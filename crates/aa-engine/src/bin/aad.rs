//! `aad`: the agentic-anything engine daemon.
//!
//! Serves `agent.sock` and `operator.sock` under the runtime dir until SIGINT/SIGTERM or the operator's
//! `Shutdown` RPC. Exit status: 0 on clean shutdown, 1 on a startup or serve error, 2 on bad arguments.

use clap::Parser as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = aa_engine::AadArgs::parse();
    let filter = args
        .log
        .clone()
        .or_else(|| std::env::var("AA_LOG").ok())
        .unwrap_or_else(|| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(&filter).unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .try_init();
    let cfg = match aa_engine::EngineConfig::from_env_and_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aad: {}\n  remediation: {}", e.message, e.remediation);
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("aad: cannot start the async runtime: {e}");
            return ExitCode::from(1);
        }
    };
    let result = runtime.block_on(async move {
        let engine = aa_engine::Engine::build(cfg, aa_engine::EngineDeps::production())?;
        engine.serve_uds(shutdown_signal()).await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aad: {}\n  remediation: {}", e.message, e.remediation);
            ExitCode::from(1)
        }
    }
}

/// SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).ok();
    let term = async {
        match term.as_mut() {
            Some(s) => {
                s.recv().await;
            }
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = term => {}
    }
}
