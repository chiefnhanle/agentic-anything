//! `aa`: the command-line client. Talks to `aad` over UDS gRPC only; never links engine internals.
//!
//! Every command is a thin client of one or two RPCs (build-plan §3.6). Output is JSON (pretty on a TTY, compact
//! when piped or with `--json`); `--full-output` wraps it in `{ok, data|error, meta:{action_id}}`. Exit codes come
//! from the single classifier in [`exit`].

pub mod commands;
pub mod connect;
pub mod exit;
pub mod output;

pub use connect::{ClientConfig, connect};
pub use exit::{CliError, exit_code, exit_code_for_error};

use clap::{CommandFactory as _, Parser, Subcommand};

/// stdin (and `@file`) input is capped at 8 MiB.
pub const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "aa",
    version,
    about = "agentic-anything client: register, distill, verify and invoke CLI capabilities through aad",
    propagate_version = true
)]
pub struct Cli {
    /// Compact JSON output (the default when stdout is not a terminal).
    #[arg(long, global = true)]
    pub json: bool,
    /// Wrap output in the envelope {ok, data|error, meta:{action_id}}.
    #[arg(long, global = true)]
    pub full_output: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the engine (execs the sibling `aad`).
    Daemon(commands::daemon::DaemonArgs),
    /// Engine status: version, listeners, caller role, halt state, assurance tier (GetStatus).
    Status,
    /// Pull the kill switch: cancel in-flight actions and refuse new dispatch (Halt).
    Halt(commands::engine::HaltArgs),
    /// Release the kill switch for the current epoch; operator socket only (Unhalt).
    Unhalt(commands::engine::UnhaltArgs),
    /// Register and inspect targets.
    Target {
        #[command(subcommand)]
        command: commands::target::TargetCommand,
    },
    /// Import external capability sources.
    Import {
        #[command(subcommand)]
        command: commands::target::ImportCommand,
    },
    /// Harvest a target's self-description, then compile its lock; prints candidate and withheld counts.
    Discover(commands::distill::DiscoverArgs),
    /// Run the verification lanes a target's capabilities need (Verify).
    Verify(commands::distill::VerifyArgs),
    /// List capabilities with their admission state (ListCapabilities).
    List(commands::catalog::ListArgs),
    /// Show one capability: schema, effects, binding, admission, records (DescribeCapability).
    Describe(commands::catalog::DescribeArgs),
    /// Invoke a capability with a JSON input object (Invoke).
    #[command(visible_alias = "x")]
    Invoke(commands::invoke::InvokeArgs),
    /// Read and verify the hash-chained audit log (QueryAudit / VerifyChain).
    Audit(commands::audit::AuditArgs),
}

/// Every command path `aa` accepts, e.g. `"target add"`, `"import cli-anything"`, `"x"` (aliases included).
///
/// Remediation strings authored by the engine must name one of these.
pub fn command_list() -> Vec<String> {
    fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
        for sub in cmd.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            let mut names = vec![sub.get_name().to_string()];
            names.extend(sub.get_all_aliases().map(str::to_string));
            for n in names {
                let path = if prefix.is_empty() { n } else { format!("{prefix} {n}") };
                out.push(path.clone());
                walk(sub, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&Cli::command(), "", &mut out);
    out
}

/// Parses `argv`, runs the command, and returns the process exit code.
pub async fn run(argv: Vec<std::ffi::OsString>) -> i32 {
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) => {
            let code = e.exit_code();
            return match e.print() {
                Err(io) if io.kind() == std::io::ErrorKind::BrokenPipe => exit::EXIT_EPIPE,
                _ => code,
            };
        }
    };
    let out = output::Output::new(cli.json, cli.full_output);
    let cfg = ClientConfig::from_env();
    let work = commands::dispatch(cli.command, &cfg, &out);

    let interrupt = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    // Dropping `work` drops any in-flight RPC stream; the engine records the action CANCELLED and kills its group.
    tokio::select! {
        result = work => out.emit(result),
        () = interrupt => exit::EXIT_SIGINT,
        () = terminate => exit::EXIT_SIGTERM,
    }
}
