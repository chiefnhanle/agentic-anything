//! Per-listener tower layer: looks up `req.uri().path()` in `aa_proto::RPC_POLICY` (unknown path =>
//! PERMISSION_DENIED), checks role ∈ roles and halt state (allowed_when_halted), checks peer uid from
//! UdsConnectInfo, and inserts `aa_core::pb::Principal` into request extensions. Errors carry an encoded aa
//! Error in Status details.
//!
//! The role is a property of the listener, never of the request (ADR 0005). A denied call to an `audited`
//! RPC is itself audited (an AdminEvent naming the path and the denial).

use crate::Engine;
use aa_core::{AaError, pb};
use std::{
    sync::Arc,
    task::{Context, Poll},
};
use tonic::transport::server::UdsConnectInfo;

/// How the peer is authenticated on this listener.
#[derive(Debug, Clone)]
pub(crate) enum PeerRule {
    /// UDS listener: `UdsConnectInfo.peer_cred.uid` must be one of these.
    Uids(Vec<u32>),
    /// In-process transport (tests / embedded): there is no peer; the caller chose the role explicitly.
    InProcess,
}

/// Halt handle and allowed uids are private; construct with `RoleLayer::new` (crate-internal).
#[derive(Clone)]
pub struct RoleLayer {
    pub role: aa_core::pb::Role,
    channel: pb::Channel,
    peer: PeerRule,
    engine: Arc<Engine>,
}

impl RoleLayer {
    pub(crate) fn new(role: pb::Role, channel: pb::Channel, peer: PeerRule, engine: Arc<Engine>) -> Self {
        Self {
            role,
            channel,
            peer,
            engine,
        }
    }
}

impl<S> tower::Layer<S> for RoleLayer {
    type Service = RoleService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RoleService {
            inner,
            layer: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RoleService<S> {
    inner: S,
    layer: RoleLayer,
}

/// `PERMISSION_DENIED` / `UNAVAILABLE` with the aa `Error` encoded in the details.
pub(crate) fn status_for(code: tonic::Code, err: &AaError) -> tonic::Status {
    let bytes = prost::Message::encode_to_vec(&err.to_proto());
    tonic::Status::with_details(code, err.message.clone(), bytes.into())
}

fn access_denied(message: String) -> AaError {
    AaError::new(
        pb::ErrorCode::AccessDenied,
        message,
        "use the listener for your role: operator RPCs need operator.sock (aa unhalt / aa approve)",
    )
}

impl<S> RoleService<S> {
    /// The listener/role/halt/peer decision for one request. `Ok` carries the principal to insert.
    fn admit(
        &self,
        path: &str,
        ext: &http::Extensions,
        headers: &http::HeaderMap,
    ) -> Result<pb::Principal, tonic::Status> {
        let l = &self.layer;
        let mut principal = pb::Principal {
            role: l.role as i32,
            channel: l.channel as i32,
            client_name: header(headers, "x-aa-client-name"),
            client_version: header(headers, "x-aa-client-version"),
            mcp_client: header(headers, "x-aa-mcp-client"),
            ..Default::default()
        };
        // Peer credentials first: an unauthenticated peer learns nothing else.
        match &l.peer {
            PeerRule::InProcess => {
                principal.uid = l.engine.owner_uid;
                principal.pid = i32::try_from(std::process::id()).unwrap_or(0);
            }
            PeerRule::Uids(allowed) => {
                let cred = ext.get::<UdsConnectInfo>().and_then(|c| c.peer_cred);
                let Some(cred) = cred else {
                    return Err(self.deny(path, &principal, access_denied("peer credentials unavailable".into())));
                };
                principal.uid = cred.uid();
                principal.pid = cred.pid().unwrap_or(0);
                if !allowed.contains(&cred.uid()) {
                    let msg = format!("uid {} is not allowed on this listener", cred.uid());
                    return Err(self.deny(path, &principal, access_denied(msg)));
                }
            }
        }
        let Some(row) = aa_proto::rpc_policy(path) else {
            return Err(self.deny(path, &principal, access_denied(format!("unknown RPC {path}"))));
        };
        if !row.roles.contains(&(l.role as i32)) {
            let err = access_denied(format!("{path} is not available to {}", l.role.as_str_name()));
            return Err(if row.audited {
                self.deny(path, &principal, err)
            } else {
                status_for(tonic::Code::PermissionDenied, &err)
            });
        }
        if !row.allowed_when_halted && l.engine.halt.is_halted() {
            let h = l.engine.halt.current();
            let err = AaError::new(
                pb::ErrorCode::EngineHalted,
                format!("the engine is halted (epoch {}); {path} is refused", h.halt_epoch),
                format!("operator: aa unhalt --epoch {}", h.halt_epoch),
            )
            .with_recovery(pb::RecoveryKind::RestartEngine, "aa unhalt");
            return Err(status_for(tonic::Code::Unavailable, &err));
        }
        Ok(principal)
    }

    /// Builds the PERMISSION_DENIED status; audited RPCs also get an AdminEvent.
    fn deny(&self, path: &str, principal: &pb::Principal, err: AaError) -> tonic::Status {
        if aa_proto::rpc_policy(path).is_some_and(|r| r.audited) {
            let details =
                serde_json::json!({ "rpc": path, "denied": "ERROR_CODE_ACCESS_DENIED", "message": err.message });
            if let Err(e) = self
                .layer
                .engine
                .admin(principal, pb::AdminEventKind::Unspecified, path, details)
            {
                tracing::error!(error = %e, "could not audit a denied RPC");
            }
        }
        status_for(tonic::Code::PermissionDenied, &err)
    }
}

fn header(h: &http::HeaderMap, name: &str) -> String {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(128).collect())
        .unwrap_or_default()
}

impl<S, B> tower::Service<http::Request<B>> for RoleService<S>
where
    S: tower::Service<http::Request<B>, Response = http::Response<tonic::body::Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: http::Request<B>) -> Self::Future {
        let path = req.uri().path().to_string();
        match self.admit(&path, req.extensions(), req.headers()) {
            Ok(principal) => {
                req.extensions_mut().insert(principal);
                // The readied service handles this call; leave a fresh clone for the next one.
                let clone = self.inner.clone();
                let mut inner = std::mem::replace(&mut self.inner, clone);
                Box::pin(async move { inner.call(req).await })
            }
            Err(status) => {
                let resp = status.into_http::<tonic::body::Body>();
                Box::pin(async move { Ok(resp) })
            }
        }
    }
}

/// The principal the role layer attached, or PERMISSION_DENIED (a service is never reachable without it).
pub(crate) fn principal_of<T>(req: &tonic::Request<T>) -> Result<pb::Principal, tonic::Status> {
    req.extensions().get::<pb::Principal>().cloned().ok_or_else(|| {
        status_for(
            tonic::Code::PermissionDenied,
            &access_denied("request did not pass the role layer".into()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, decode_status};
    use std::convert::Infallible;
    use tower::{Layer, ServiceExt};

    /// Echoes the principal role it saw, so tests can check the extension was inserted.
    fn echo() -> impl tower::Service<
        http::Request<()>,
        Response = http::Response<tonic::body::Body>,
        Error = Infallible,
        Future = impl Send,
    > + Clone
    + Send
    + 'static {
        tower::service_fn(|req: http::Request<()>| async move {
            let p = req
                .extensions()
                .get::<pb::Principal>()
                .cloned()
                .expect("principal inserted");
            let mut resp = http::Response::new(tonic::body::Body::default());
            resp.headers_mut().insert("x-role", http::HeaderValue::from(p.role));
            resp.headers_mut().insert("x-uid", http::HeaderValue::from(p.uid));
            Ok::<_, Infallible>(resp)
        })
    }

    fn request(path: &str, cred: Option<UdsConnectInfo>) -> http::Request<()> {
        let mut req = http::Request::builder().uri(path).body(()).unwrap();
        if let Some(c) = cred {
            req.extensions_mut().insert(c);
        }
        req
    }

    /// A real UCred for this process (UCred has no public constructor), taken from a socket pair.
    fn own_cred() -> UdsConnectInfo {
        let (a, _b) = tokio::net::UnixStream::pair().unwrap();
        let info = tonic::transport::server::Connected::connect_info(&a);
        assert!(info.peer_cred.is_some());
        info
    }

    async fn call(layer: &RoleLayer, req: http::Request<()>) -> http::Response<tonic::body::Body> {
        layer.layer(echo()).oneshot(req).await.unwrap()
    }

    fn grpc_status(resp: &http::Response<tonic::body::Body>) -> Option<tonic::Status> {
        tonic::Status::from_header_map(resp.headers())
    }

    #[tokio::test]
    async fn peer_uid_not_allowed_is_permission_denied() {
        let h = Harness::new();
        let cred = own_cred();
        let my_uid = cred.peer_cred.unwrap().uid();
        let path = "/agentic_anything.v1.EngineService/GetStatus";

        let allowed = RoleLayer::new(
            pb::Role::Agent,
            pb::Channel::AgentSocket,
            PeerRule::Uids(vec![my_uid]),
            h.engine.clone(),
        );
        let resp = call(&allowed, request(path, Some(cred.clone()))).await;
        assert!(grpc_status(&resp).is_none(), "allowed uid passes");
        assert_eq!(resp.headers()["x-role"], (pb::Role::Agent as i32).to_string().as_str());
        assert_eq!(resp.headers()["x-uid"], my_uid.to_string().as_str());

        let other = RoleLayer::new(
            pb::Role::Agent,
            pb::Channel::AgentSocket,
            PeerRule::Uids(vec![my_uid.wrapping_add(1)]),
            h.engine.clone(),
        );
        let st = grpc_status(&call(&other, request(path, Some(cred))).await).unwrap();
        assert_eq!(st.code(), tonic::Code::PermissionDenied);
        assert_eq!(decode_status(&st).code(), pb::ErrorCode::AccessDenied);

        // No peer credentials at all on a UDS listener: denied.
        let st = grpc_status(&call(&allowed, request(path, None)).await).unwrap();
        assert_eq!(st.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn unknown_path_and_wrong_role() {
        let h = Harness::new();
        let agent = RoleLayer::new(
            pb::Role::Agent,
            pb::Channel::InProcess,
            PeerRule::InProcess,
            h.engine.clone(),
        );
        let st = grpc_status(&call(&agent, request("/agentic_anything.v1.Nope/X", None)).await).unwrap();
        assert_eq!(st.code(), tonic::Code::PermissionDenied);

        let before = h.entries().len();
        let st = grpc_status(&call(&agent, request("/agentic_anything.v1.EngineService/Unhalt", None)).await).unwrap();
        assert_eq!(st.code(), tonic::Code::PermissionDenied);
        assert_eq!(decode_status(&st).code(), pb::ErrorCode::AccessDenied);
        // Unhalt is audited: the denial is an AdminEvent.
        let entries = h.entries();
        assert_eq!(entries.len(), before + 1);
        match &entries.last().unwrap().body {
            Some(pb::audit_entry::Body::Admin(a)) => {
                assert_eq!(a.subject, "/agentic_anything.v1.EngineService/Unhalt")
            }
            other => panic!("expected an admin event, got {other:?}"),
        }

        // ENGINE_INTERNAL-only adapter RPCs are unreachable from both listeners.
        let op = RoleLayer::new(
            pb::Role::Operator,
            pb::Channel::InProcess,
            PeerRule::InProcess,
            h.engine.clone(),
        );
        let st = grpc_status(&call(&op, request("/agentic_anything.v1.AdapterService/Invoke", None)).await).unwrap();
        assert_eq!(st.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn halted_engine_refuses_non_halt_rpcs() {
        let h = Harness::new();
        h.engine
            .halt
            .halt(pb::HaltSource::Rpc, pb::HaltScope::Engine, "", "test");
        let agent = RoleLayer::new(
            pb::Role::Agent,
            pb::Channel::InProcess,
            PeerRule::InProcess,
            h.engine.clone(),
        );
        let st = grpc_status(&call(&agent, request("/agentic_anything.v1.RuntimeService/Invoke", None)).await).unwrap();
        assert_eq!(st.code(), tonic::Code::Unavailable);
        let e = decode_status(&st);
        assert_eq!(e.code(), pb::ErrorCode::EngineHalted);
        assert_eq!(e.recovery.unwrap().kind(), pb::RecoveryKind::RestartEngine);
        let resp = call(&agent, request("/agentic_anything.v1.EngineService/GetStatus", None)).await;
        assert!(grpc_status(&resp).is_none(), "GetStatus is allowed when halted");
    }
}
