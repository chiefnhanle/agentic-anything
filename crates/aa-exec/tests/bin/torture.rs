//! Torture helper for the supervision tests (never shipped; `aa-exec` test helper binary).
//!
//! Modes:
//! - `ignore-term <secs>`: ignores SIGTERM (a tokio signal listener swallows it) and sleeps;
//! - `grandchild <pidfile>`: spawns a grandchild (`sleep 60`, same process group), writes its pid, exits 0;
//! - `tree <pidfile>`: spawns a grandchild, writes "<own pid> <grandchild pid>", then sleeps 60 s;
//! - `flood <bytes>`: writes `<bytes>` bytes to stdout;
//! - `stderr-token`: prints a fake token to stderr and exits 3;
//! - `sleep <secs>`: sleeps;
//! - `write-home <rel> [args…]`: claims to be a read but writes `$HOME/<rel>` (like `notes stats`), exits 0;
//! - `strict-sep [args…]`: a parser that rejects `--opt=value` and unknown flags with exit 2 but accepts
//!   `--opt value` (exercises L1's `no_equals_form` detection).

use std::{io::Write, time::Duration};

fn spawn_grandchild() -> std::io::Result<std::process::Child> {
    let me = std::env::current_exe()?;
    std::process::Command::new(me).args(["sleep", "60"]).spawn()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    match arg(0).as_str() {
        "ignore-term" => {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let secs: u64 = arg(1).parse().unwrap_or(60);
            let sleep = tokio::time::sleep(Duration::from_secs(secs));
            tokio::pin!(sleep);
            println!("ready");
            loop {
                tokio::select! {
                    _ = term.recv() => eprintln!("torture: ignoring SIGTERM"),
                    () = &mut sleep => break,
                }
            }
        }
        "grandchild" => {
            let child = spawn_grandchild()?;
            std::fs::write(arg(1), child.id().to_string())?;
            // Exit without waiting: the grandchild keeps our stdout open and stays in the group.
            std::mem::forget(child);
        }
        "tree" => {
            let child = spawn_grandchild()?;
            std::fs::write(arg(1), format!("{} {}", std::process::id(), child.id()))?;
            std::mem::forget(child);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        "flood" => {
            let total: u64 = arg(1).parse().unwrap_or(0);
            let chunk = vec![b'x'; 64 * 1024];
            let mut out = std::io::stdout().lock();
            let mut left = total;
            while left > 0 {
                let n = left.min(chunk.len() as u64) as usize;
                if out.write_all(&chunk[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
        }
        "stderr-token" => {
            eprintln!("error: authentication failed for token ghp_abcdefghijklmnopqrstuvwxyz0123456789");
            std::process::exit(3);
        }
        "write-home" => {
            let home = std::env::var("HOME").unwrap_or_default();
            let path = std::path::Path::new(&home).join(arg(1));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, b"stats")?;
            println!("ok");
        }
        "strict-sep" => {
            let mut rest = args[1..].iter();
            while let Some(a) = rest.next() {
                if a == "--opt" {
                    rest.next();
                } else if a.starts_with("--") {
                    eprintln!("Error: No such option: {a}");
                    std::process::exit(2);
                }
            }
            println!("ok");
        }
        "sleep" => {
            let secs: u64 = arg(1).parse().unwrap_or(60);
            std::thread::sleep(Duration::from_secs(secs));
        }
        other => {
            eprintln!("torture: unknown mode {other:?}");
            std::process::exit(64);
        }
    }
    Ok(())
}
