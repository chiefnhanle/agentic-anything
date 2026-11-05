//! `aa target add|list|show` and `aa import cli-anything` (TargetService).

use super::{absolute, agent, engine::get_status, valid_target_id};
use crate::connect::ClientConfig;
use crate::exit::CliError;
use crate::output::{CmdResult, Failure, Reply, to_json};
use aa_proto::v1 as pb;
use pb::target_service_client::TargetServiceClient;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Overlays are small, human-authored YAML files.
const MAX_OVERLAY_BYTES: u64 = 1024 * 1024;
const OVERLAY_FILE: &str = "target.aa.yaml";

#[derive(Debug, clap::Subcommand)]
pub enum TargetCommand {
    /// Register a target: pin its identity and record the authorization statement (CreateTarget).
    Add(TargetAddArgs),
    /// List registered targets (ListTargets).
    #[command(visible_alias = "ls")]
    List,
    /// Show one target and its lock digest (GetTarget).
    Show {
        /// Target id, e.g. `git`.
        target: String,
    },
}

#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("locator").required(true).args(["exec", "python_script"])))]
pub struct TargetAddArgs {
    /// Target id, e.g. `git` (`[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*`).
    pub id: String,
    /// Executable: a name on PATH or a path.
    #[arg(long)]
    pub exec: Option<PathBuf>,
    /// A Python console script, e.g. `<venv>/bin/notes-cli` (Click/CLI-Anything).
    #[arg(long)]
    pub python_script: Option<PathBuf>,
    /// Harvester to run later (repeatable), e.g. `cobra-complete`; default: auto.
    #[arg(long = "harvester")]
    pub harvesters: Vec<String>,
    /// Overlay file (`target.aa.yaml`) to install for this target.
    #[arg(long)]
    pub overlay: Option<PathBuf>,
    /// Why you may automate this target (required; recorded verbatim in the audit log).
    #[arg(long, required = true)]
    pub authorization: String,
}

#[derive(Debug, clap::Subcommand)]
pub enum ImportCommand {
    /// Import CLI-Anything harnesses from a local checkout (STATIC; never runs install_cmd).
    #[command(name = "cli-anything")]
    CliAnything(ImportCliAnythingArgs),
}

#[derive(Debug, clap::Args)]
pub struct ImportCliAnythingArgs {
    /// Path of the local CLI-Anything checkout (contains registry.json).
    pub repo: PathBuf,
    /// Harness name from registry.json (repeatable); none = list the available harnesses.
    #[arg(long = "harness")]
    pub harnesses: Vec<String>,
    /// Interpreter that already has the harness installed (an operator-installed venv).
    #[arg(long)]
    pub python: Option<PathBuf>,
    /// Why you may automate these harnesses (required; recorded verbatim in the audit log).
    #[arg(long, required = true)]
    pub authorization: String,
}

pub async fn run_target(cmd: TargetCommand, cfg: &ClientConfig) -> CmdResult {
    match cmd {
        TargetCommand::Add(a) => add(a, cfg).await,
        TargetCommand::List => {
            let mut c = TargetServiceClient::new(agent(cfg).await?);
            let mut targets = Vec::new();
            let mut offset = 0u32;
            loop {
                let r = c
                    .list_targets(pb::ListTargetsRequest {
                        page: Some(pb::PageRequest { offset, limit: 0 }),
                    })
                    .await?
                    .into_inner();
                targets.extend(r.targets.iter().map(to_json));
                match r.page {
                    Some(p) if p.next_offset > offset && !r.targets.is_empty() => offset = p.next_offset,
                    _ => break,
                }
            }
            Ok(Reply::new(serde_json::json!({ "targets": targets })))
        }
        TargetCommand::Show { target } => {
            let mut c = TargetServiceClient::new(agent(cfg).await?);
            let r = c
                .get_target(pb::GetTargetRequest { target_id: target })
                .await?
                .into_inner();
            Ok(Reply::new(to_json(&r)))
        }
    }
}

fn require_authorization(s: &str) -> Result<String, CliError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(CliError::Usage("--authorization must be a non-empty statement".into()));
    }
    Ok(t.to_string())
}

/// `--exec git` stays a PATH name; anything with a `/` is made absolute against the client's cwd.
fn exec_locator(p: &Path) -> Result<String, CliError> {
    let s = p.to_string_lossy();
    if s.is_empty() {
        return Err(CliError::Usage("--exec must not be empty".into()));
    }
    if s.contains('/') {
        Ok(absolute(p)?.to_string_lossy().into_owned())
    } else {
        Ok(s.into_owned())
    }
}

pub(crate) fn read_capped(path: &Path, cap: u64, what: &str) -> Result<Vec<u8>, CliError> {
    use std::io::Read as _;
    let f = std::fs::File::open(path)
        .map_err(|e| CliError::Usage(format!("cannot open {what} {}: {e}", path.display())))?;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| CliError::Usage(format!("cannot read {what} {}: {e}", path.display())))?;
    if buf.len() as u64 > cap {
        return Err(CliError::Usage(format!(
            "{what} {} exceeds {cap} bytes",
            path.display()
        )));
    }
    Ok(buf)
}

/// The top-level `target:` value of an overlay (light check; the engine parses the overlay for real).
pub(crate) fn overlay_target(yaml: &str) -> Option<String> {
    yaml.lines().find_map(|line| {
        let rest = line.strip_prefix("target:")?;
        let v = match rest.find(" #") {
            Some(i) => &rest[..i],
            None => rest,
        };
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|x| x.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .unwrap_or(v);
        Some(v.to_string())
    })
}

/// Atomic write (temp file in the same dir, 0600, fsync, rename); creates missing parents 0700.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("overlay path has no parent"))?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

async fn add(a: TargetAddArgs, cfg: &ClientConfig) -> CmdResult {
    if !valid_target_id(&a.id) {
        return Err(CliError::Usage(format!(
            "invalid target id {:?}: expected [a-z0-9][a-z0-9-]*(.[a-z0-9-]+)*",
            a.id
        ))
        .into());
    }
    let authorization = require_authorization(&a.authorization)?;
    let locator = match (&a.exec, &a.python_script) {
        (Some(e), None) => pb::target_locator::Locator::Executable(exec_locator(e)?),
        (None, Some(p)) => {
            pb::target_locator::Locator::PythonConsoleScript(absolute(p)?.to_string_lossy().into_owned())
        }
        _ => return Err(CliError::Usage("give exactly one of --exec or --python-script".into()).into()),
    };

    // The overlay travels as a file in the engine's workspace: CreateTarget keeps an existing overlay.
    let overlay = match &a.overlay {
        Some(p) => {
            let bytes = read_capped(p, MAX_OVERLAY_BYTES, "overlay")?;
            let text = String::from_utf8(bytes)
                .map_err(|_| CliError::Usage(format!("overlay {} is not UTF-8", p.display())))?;
            match overlay_target(&text) {
                Some(t) if t == a.id => {}
                Some(t) => {
                    return Err(CliError::Usage(format!(
                        "overlay {} is for target {t:?}, not {:?}",
                        p.display(),
                        a.id
                    ))
                    .into());
                }
                None => {
                    return Err(
                        CliError::Usage(format!("overlay {} has no top-level `target:` key", p.display())).into(),
                    );
                }
            }
            let status = get_status(cfg).await?;
            if status.workspace_root.is_empty() {
                return Err(CliError::Usage(
                    "the engine did not report its workspace root; cannot install --overlay".into(),
                )
                .into());
            }
            let dest = PathBuf::from(&status.workspace_root)
                .join("targets")
                .join(&a.id)
                .join(OVERLAY_FILE);
            let created_dir = dest.parent().filter(|d| !d.exists()).map(Path::to_path_buf);
            write_atomic(&dest, text.as_bytes())
                .map_err(|e| CliError::Usage(format!("cannot install overlay at {}: {e}", dest.display())))?;
            Some((text, dest, created_dir))
        }
        None => None,
    };

    let created = async {
        let mut c = TargetServiceClient::new(agent(cfg).await?);
        let resp = c
            .create_target(pb::CreateTargetRequest {
                target_id: a.id.clone(),
                locator: Some(pb::TargetLocator { locator: Some(locator) }),
                authorization_statement: authorization,
                harvesters: a.harvesters.clone(),
            })
            .await?
            .into_inner();
        Ok::<_, CliError>(resp)
    }
    .await;
    let resp = match created {
        Ok(r) => r,
        Err(e) => {
            // Do not leave a bare target dir (overlay only) behind for a registration that did not happen.
            if let Some((_, _, Some(dir))) = &overlay {
                let _ = std::fs::remove_dir_all(dir);
            }
            return Err(e.into());
        }
    };

    let mut data = to_json(&resp);
    if let Some((text, dest, _)) = overlay {
        // If the engine keeps the overlay elsewhere, or replaced ours with a starter, put ours where it reads it.
        let engine_path = if resp.overlay_path.is_empty() {
            dest.clone()
        } else {
            PathBuf::from(&resp.overlay_path)
        };
        let current = std::fs::read_to_string(&engine_path).ok();
        if current.as_deref() != Some(text.as_str()) {
            write_atomic(&engine_path, text.as_bytes())
                .map_err(|e| CliError::Usage(format!("cannot install overlay at {}: {e}", engine_path.display())))?;
        }
        data["overlayInstalled"] = serde_json::Value::String(engine_path.to_string_lossy().into_owned());
    }
    Ok(Reply::new(data))
}

pub async fn run_import(cmd: ImportCommand, cfg: &ClientConfig) -> CmdResult {
    let ImportCommand::CliAnything(a) = cmd;
    let authorization = require_authorization(&a.authorization)?;
    if !a.harnesses.is_empty() && a.python.is_none() {
        return Err(CliError::Usage("--python <venv-python> is required when importing a harness".into()).into());
    }
    let req = pb::ImportCliAnythingRequest {
        repo_path: absolute(&a.repo)?.to_string_lossy().into_owned(),
        harnesses: a.harnesses,
        python: match &a.python {
            Some(p) => absolute(p)?.to_string_lossy().into_owned(),
            None => String::new(),
        },
        authorization_statement: authorization,
    };
    let mut c = TargetServiceClient::new(agent(cfg).await?);
    let resp = c.import_cli_anything(req).await?.into_inner();
    if let Some(err) = resp
        .harnesses
        .iter()
        .find_map(|h| h.error.clone().filter(|e| e.code != 0))
    {
        return Err(Failure::from(CliError::Domain(err)));
    }
    Ok(Reply::new(to_json(&resp)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_target_key() {
        assert_eq!(
            overlay_target("schema: aa.overlay/v1\ntarget: git\n").as_deref(),
            Some("git")
        );
        assert_eq!(overlay_target("target: \"git\"  # c\n").as_deref(), Some("git"));
        assert_eq!(
            overlay_target("target: 'fixture-notes'\n").as_deref(),
            Some("fixture-notes")
        );
        // Nested keys are not the top-level target.
        assert_eq!(overlay_target("x:\n  target: y\n"), None);
    }

    #[test]
    fn exec_locator_keeps_path_names() {
        assert_eq!(exec_locator(Path::new("git")).unwrap(), "git");
        assert_eq!(exec_locator(Path::new("/usr/bin/git")).unwrap(), "/usr/bin/git");
        let rel = exec_locator(Path::new("./bin/x")).unwrap();
        assert!(rel.starts_with('/') && rel.ends_with("bin/x"), "{rel}");
    }

    #[test]
    fn atomic_write_creates_private_parents() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("targets/git/target.aa.yaml");
        write_atomic(&p, b"a").unwrap();
        write_atomic(&p, b"b").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"b");
        let mode = std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(
            std::fs::read_dir(p.parent().unwrap()).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }

    #[test]
    fn capped_reads() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        std::fs::write(&p, vec![b'x'; 11]).unwrap();
        assert!(read_capped(&p, 10, "input").is_err());
        assert_eq!(read_capped(&p, 11, "input").unwrap().len(), 11);
        assert!(read_capped(&d.path().join("missing"), 10, "input").is_err());
    }

    #[test]
    fn authorization_must_be_non_empty() {
        assert!(require_authorization("  ").is_err());
        assert_eq!(require_authorization(" owner ").unwrap(), "owner");
    }
}
