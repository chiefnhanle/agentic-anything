//! Listeners (architecture §5.1, ADR 0005): two UDS sockets, one role each, plus an in-process duplex
//! transport for tests.
//!
//! Startup: refuse euid 0; create/check the runtime dir (0700, owned, not a symlink); take
//! `flock(engine.lock)`; unlink stale sockets; bind `agent.sock` and `operator.sock`; chmod 0600; re-lstat.

use crate::{Engine, config::config_error, role_layer::PeerRule, role_layer::RoleLayer, services};
use aa_core::{AaError, AaResult, pb};
use std::{
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;

pub(crate) const AGENT_SOCK: &str = "agent.sock";
pub(crate) const OPERATOR_SOCK: &str = "operator.sock";
const LOCK_FILE: &str = "engine.lock";

/// Resolves when shutdown has been requested on `rx`.
async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Creates the runtime dir 0700 if missing; refuses a symlink, a foreign owner or group/other bits.
pub(crate) fn check_runtime_dir(dir: &Path, uid: u32) -> AaResult<()> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::DirBuilder::new().mode(0o700).create(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let md = std::fs::symlink_metadata(dir)?;
    if md.file_type().is_symlink() || !md.is_dir() {
        return Err(config_error(format!(
            "runtime dir {} is not a real directory",
            dir.display()
        )));
    }
    if md.uid() != uid {
        return Err(config_error(format!(
            "runtime dir {} is owned by uid {}, not {uid}",
            dir.display(),
            md.uid()
        )));
    }
    if md.mode() & 0o077 != 0 {
        return Err(config_error(format!(
            "runtime dir {} has group/other permissions ({:o}); chmod 700 it",
            dir.display(),
            md.mode() & 0o777
        )));
    }
    Ok(())
}

fn bind(path: &Path, uid: u32) -> AaResult<tokio::net::UnixListener> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_socket() => std::fs::remove_file(path)?,
        Ok(_) => {
            return Err(config_error(format!(
                "{} exists and is not a socket; refusing to replace it",
                path.display()
            )));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let md = std::fs::symlink_metadata(path)?;
    if !md.file_type().is_socket() || md.uid() != uid || md.mode() & 0o077 != 0 {
        return Err(config_error(format!("{} failed the post-bind check", path.display())));
    }
    Ok(listener)
}

pub(crate) async fn serve_uds(
    engine: Arc<Engine>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> AaResult<()> {
    let uid = engine.owner_uid;
    if uid == 0 {
        return Err(config_error("aad refuses to run as root"));
    }
    let dir = engine.cfg.runtime_dir.clone();
    check_runtime_dir(&dir, uid)?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    let mut lock = fd_lock::RwLock::new(lock_file);
    let _guard = lock.try_write().map_err(|_| {
        AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!("another aad holds {}", dir.join(LOCK_FILE).display()),
            "aa status (an engine is already running for this runtime dir)",
        )
    })?;
    let agent_path = dir.join(AGENT_SOCK);
    let operator_path = dir.join(OPERATOR_SOCK);
    let agent = bind(&agent_path, uid)?;
    let operator = bind(&operator_path, uid)?;
    *crate::status::lock(&engine.listen) = (uri(&agent_path), uri(&operator_path));
    tracing::info!(agent = %agent_path.display(), operator = %operator_path.display(), "aad listening");

    // The caller's shutdown future and the Shutdown RPC both flip the same switch.
    let e2 = Arc::clone(&engine);
    let external = tokio::spawn(async move {
        shutdown.await;
        e2.request_shutdown();
    });
    let watchers = tokio::spawn(halt_watchers(Arc::clone(&engine)));

    let agent_layer = RoleLayer::new(
        pb::Role::Agent,
        pb::Channel::AgentSocket,
        PeerRule::Uids(engine.cfg.agent_uids.clone()),
        Arc::clone(&engine),
    );
    let operator_layer = RoleLayer::new(
        pb::Role::Operator,
        pb::Channel::OperatorSocket,
        PeerRule::Uids(vec![uid]),
        Arc::clone(&engine),
    );
    let agent_srv = services::router(Arc::clone(&engine), agent_layer).serve_with_incoming_shutdown(
        tokio_stream::wrappers::UnixListenerStream::new(agent),
        wait_shutdown(engine.shutdown.subscribe()),
    );
    let operator_srv = services::router(Arc::clone(&engine), operator_layer).serve_with_incoming_shutdown(
        tokio_stream::wrappers::UnixListenerStream::new(operator),
        wait_shutdown(engine.shutdown.subscribe()),
    );
    let (a, o) = tokio::join!(agent_srv, operator_srv);
    external.abort();
    watchers.abort();
    for p in [&agent_path, &operator_path] {
        let _ = std::fs::remove_file(p);
    }
    let sys = engine.system_principal();
    if let Err(e) = engine.admin(
        &sys,
        pb::AdminEventKind::EngineStop,
        "aad",
        serde_json::json!({"halted": engine.halt.is_halted()}),
    ) {
        tracing::error!(error = %e, "could not record ENGINE_STOP");
    }
    a.and(o)
        .map_err(|e| AaError::internal(format!("gRPC server failed: {e}")))
}

fn uri(p: &Path) -> String {
    format!("unix://{}", p.display())
}

/// The `state/HALT` sentinel (polled) and SIGUSR1 both engage the kill switch.
async fn halt_watchers(engine: Arc<Engine>) {
    let sys = engine.system_principal();
    let mut usr1 = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).ok();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        let signalled = async {
            match usr1.as_mut() {
                Some(s) => s.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tick.tick() => {
                if !engine.halt.is_halted() && engine.halt.sentinel_present() {
                    engine.engage_halt(&sys, pb::HaltSource::SentinelFile, pb::HaltScope::Engine, "", "HALT sentinel file");
                }
            }
            Some(()) = signalled => {
                engine.engage_halt(&sys, pb::HaltSource::Signal, pb::HaltScope::Engine, "", "SIGUSR1");
            }
        }
    }
}

pub(crate) async fn connect_in_process(engine: &Arc<Engine>, role: pb::Role) -> AaResult<tonic::transport::Channel> {
    let (client, server) = tokio::io::duplex(1 << 20);
    let layer = RoleLayer::new(role, pb::Channel::InProcess, PeerRule::InProcess, Arc::clone(engine));
    let router = services::router(Arc::clone(engine), layer);
    let stop = wait_shutdown(engine.shutdown.subscribe());
    tokio::spawn(async move {
        // One connection, then stay open: an incoming stream that ends makes tonic drain and GOAWAY.
        let incoming = tokio_stream::StreamExt::chain(
            tokio_stream::once(Ok::<_, std::io::Error>(server)),
            tokio_stream::pending(),
        );
        if let Err(e) = router.serve_with_incoming_shutdown(incoming, stop).await {
            tracing::warn!(error = %e, "in-process server ended");
        }
    });
    let mut client = Some(client);
    tonic::transport::Endpoint::from_static("http://in-process.aa.invalid")
        .connect_with_connector(tower::service_fn(move |_: http::Uri| {
            let c = client.take();
            async move {
                c.map(hyper_util::rt::TokioIo::new)
                    .ok_or_else(|| std::io::Error::other("the in-process transport was already used"))
            }
        }))
        .await
        .map_err(|e| AaError::internal(format!("in-process connect failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_dir_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = crate::config::euid();
        let dir = tmp.path().join("rt");
        check_runtime_dir(&dir, uid).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        check_runtime_dir(&dir, uid).unwrap();
        assert!(check_runtime_dir(&dir, uid.wrapping_add(1)).is_err(), "foreign owner");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_runtime_dir(&dir, uid).is_err(), "group/other bits");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(check_runtime_dir(&link, uid).is_err(), "symlink");
    }
}
