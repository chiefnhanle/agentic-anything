//! CLI-Anything registry reading and STATIC harness import (ADR 0009, architecture §11). `install_cmd` is stored
//! verbatim and never executed; the harness SKILL.md is stored as untrusted reference text and never served.
//!
//! Import reads files only: `registry.json`, the harness directory (hashed into `source_tree_sha`), the
//! LICENSE, and the operator-installed interpreter / entry module / backend binaries (hashed into the pinned
//! identity). It never runs Python, git, pip or the backend.

use crate::common::{self};
use aa_core::{
    AaError, AaResult,
    api::{Candidate, HarvestCtx, Harvester, RawHarvest},
    digest, ids, pb,
};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::LazyLock,
};

/// Raw artifact names produced by an import (and by [`CliAnythingRegistryHarvester::harvest`]).
pub const RAW_REGISTRY_ENTRY: &str = "registry-entry.json";
/// The registry `install_cmd`, verbatim. Never executed.
pub const RAW_INSTALL_CMD: &str = "install_cmd";
/// The harness SKILL.md: untrusted reference text, never served.
pub const RAW_SKILL_MD: &str = "SKILL.md";
/// The sorted blake3 manifest of the harness tree whose digest is `source_tree_sha`.
pub const RAW_TREE_MANIFEST: &str = "harness-tree.manifest";

const HARVESTER: &str = "cli-anything-registry@1";

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub requires: Option<String>,
    pub homepage: Option<String>,
    pub source_url: Option<String>,
    pub install_cmd: Option<String>,
    pub entry_point: String,
    pub skill_md: Option<String>,
    pub category: Option<String>,
}

fn registry_path(repo: &Path) -> PathBuf {
    repo.join("registry.json")
}

fn read_registry_values(repo: &Path) -> AaResult<Vec<Value>> {
    let path = registry_path(repo);
    let bytes = std::fs::read(&path).map_err(|e| {
        AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!("cannot read {}: {e}", path.display()),
            "point at a local CLI-Anything checkout: aa import cli-anything <repo> --harness <name> …",
        )
    })?;
    let doc: Value = serde_json::from_slice(&bytes)?;
    match doc.get("clis") {
        Some(Value::Array(a)) => Ok(a.clone()),
        _ => Err(AaError::invalid(format!("{} has no `clis` array", path.display()))),
    }
}

pub fn read_registry(repo: &std::path::Path) -> AaResult<Vec<RegistryEntry>> {
    read_registry_values(repo)?
        .into_iter()
        .map(|v| {
            serde_json::from_value::<RegistryEntry>(v)
                .map_err(|e| AaError::invalid(format!("registry.json entry does not match the registry schema: {e}")))
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ImportedHarness {
    /// id "cli-anything.<name>", kind CLI_ANYTHING_HARNESS, identity files {interpreter, entry_module,
    /// backend:<requires>?}, source_tree_sha, license.
    pub target: pb::Target,
    /// Starter: state_handle --project (engine_supplied), exclude repl, harvest [click-info].
    pub overlay_yaml: String,
    /// Registry entry JSON, install_cmd (verbatim, never run), SKILL.md (untrusted).
    pub raw: Vec<RawHarvest>,
    /// e.g. "shell-metachar", "unpinned-git-url".
    pub install_cmd_flags: Vec<String>,
}

/// `cli-anything.<name>` (lowercased, `_` -> `-`), validated.
pub(crate) fn target_id_for(name: &str) -> AaResult<String> {
    let id = format!("cli-anything.{}", name.to_ascii_lowercase().replace('_', "-"));
    ids::validate_target_id(&id)?;
    Ok(id)
}

fn find_entry(repo: &Path, name: &str) -> AaResult<(RegistryEntry, Value)> {
    let values = read_registry_values(repo)?;
    let norm = |s: &str| s.to_ascii_lowercase().replace('_', "-");
    let found = values
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(name))
        .or_else(|| {
            values.iter().find(|v| {
                v.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| norm(n) == norm(name))
            })
        });
    let Some(value) = found else {
        let mut names: Vec<&str> = values
            .iter()
            .filter_map(|v| v.get("name").and_then(Value::as_str))
            .collect();
        names.sort_unstable();
        return Err(AaError::new(
            pb::ErrorCode::UnresolvedSelector,
            format!(
                "no harness named '{name}' in {} ({} entries)",
                registry_path(repo).display(),
                names.len()
            ),
            "list the available harnesses: aa import cli-anything <repo>",
        )
        .with_recovery(
            pb::RecoveryKind::InspectInputContract,
            "TargetService.ImportCliAnything",
        ));
    };
    let entry: RegistryEntry = serde_json::from_value(value.clone())
        .map_err(|e| AaError::invalid(format!("registry entry '{name}' is malformed: {e}")))?;
    Ok((entry, value.clone()))
}

static SUBDIRECTORY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#subdirectory=([A-Za-z0-9_./-]+)").expect("static regex"));

/// The harness directory inside the checkout (realpath'd, must stay inside the repo).
fn harness_dir(repo: &Path, entry: &RegistryEntry) -> AaResult<PathBuf> {
    let repo_real = repo.canonicalize()?;
    let mut candidates = Vec::new();
    // A `#subdirectory=<x>/agent-harness` hint (CLI-Anything layout); external repos never match locally.
    if let Some(c) = entry.install_cmd.as_deref().and_then(|c| SUBDIRECTORY.captures(c))
        && c[1].ends_with("agent-harness")
    {
        candidates.push(repo.join(&c[1]));
    }
    candidates.push(repo.join(&entry.name).join("agent-harness"));
    if let Ok(rd) = std::fs::read_dir(repo) {
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&entry.name))
            .map(|e| e.path().join("agent-harness"))
            .collect();
        dirs.sort();
        candidates.extend(dirs);
    }
    for c in candidates {
        if let Ok(real) = c.canonicalize()
            && real.is_dir()
            && real.starts_with(&repo_real)
        {
            return Ok(real);
        }
    }
    Err(AaError::new(
        pb::ErrorCode::CapabilityUnavailable,
        format!(
            "harness '{}' has no agent-harness directory in this checkout (its install_cmd points elsewhere)",
            entry.name
        ),
        "import only harnesses that live in the local checkout: aa import cli-anything <repo>",
    ))
}

/// Directory/file names that are build products or caches, not harness source.
fn excluded(name: &str) -> bool {
    matches!(
        name,
        ".git" | "__pycache__" | "build" | "dist" | ".DS_Store" | ".pytest_cache" | ".venv"
    ) || name.ends_with(".egg-info")
        || name.ends_with(".pyc")
        || name.ends_with(".pyo")
}

/// Sorted blake3 manifest of the harness tree: one `<blake3> <relpath>` (or `symlink:<target> <relpath>`)
/// line per file. Build products (`build/`, `*.egg-info`, `__pycache__`) are excluded, so an operator's
/// `uv pip install <dir>` does not change the identity.
pub(crate) fn tree_manifest(dir: &Path) -> AaResult<Vec<u8>> {
    let mut lines = Vec::new();
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !excluded(&e.file_name().to_string_lossy()));
    for entry in walker {
        let entry = entry.map_err(|e| AaError::internal(format!("walking {}: {e}", dir.display())))?;
        let rel = entry
            .path()
            .strip_prefix(dir)
            .map_err(|e| AaError::internal(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let ft = entry.file_type();
        if ft.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            lines.push((rel.clone(), format!("symlink:{} {rel}", target.to_string_lossy())));
        } else if ft.is_file() {
            let bytes = std::fs::read(entry.path())?;
            lines.push((rel.clone(), format!("{} {rel}", blake3::hash(&bytes).to_hex())));
        }
    }
    lines.sort();
    let mut out = Vec::new();
    for (_, l) in lines {
        out.extend_from_slice(l.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

/// `blake3:<hex>` of the tree manifest.
pub(crate) fn source_tree_sha(manifest: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(manifest).to_hex())
}

static GIT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"git\+[a-z]+://[^\s#@]+(?:@([^\s#]+))?").expect("static regex"));
static PIP_INSTALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:python3? -m )?pip3? install\s+(.+)$").expect("static regex"));

/// Static review of a registry `install_cmd` (it is never executed).
pub(crate) fn install_cmd_flags(cmd: &str) -> Vec<String> {
    let mut flags = BTreeSet::new();
    if cmd.contains([
        ';', '|', '&', '$', '`', '<', '>', '(', ')', '{', '}', '*', '?', '!', '\n', '\r', '\\',
    ]) {
        flags.insert("shell-metachar");
    }
    if (cmd.contains("curl ") || cmd.contains("wget ")) && cmd.contains('|') {
        flags.insert("remote-script");
    }
    for c in GIT_URL.captures_iter(cmd) {
        match c.get(1) {
            None => flags.insert("unpinned-git-url"),
            Some(r) if !(r.as_str().len() == 40 && r.as_str().chars().all(|c| c.is_ascii_hexdigit())) => {
                flags.insert("mutable-git-ref")
            }
            Some(_) => false,
        };
    }
    match PIP_INSTALL.captures(cmd) {
        Some(c) => {
            for tok in c[1].split_whitespace().filter(|t| !t.starts_with('-')) {
                let is_spec = !tok.contains("://") && !tok.starts_with('.') && !tok.starts_with('/');
                if is_spec && !tok.contains("==") {
                    flags.insert("unpinned-package");
                }
            }
        }
        None => {
            flags.insert("not-pip");
        }
    }
    flags.into_iter().map(str::to_owned).collect()
}

/// Binaries named by a registry `requires` (`libreoffice` -> `soffice`, `gimp (apt install gimp)` -> `gimp`,
/// `blender >= 4.2` -> `blender`). Free text that names no binary yields nothing.
pub(crate) fn required_binaries(requires: &str) -> Vec<String> {
    static PIECE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([a-z][a-z0-9_+.-]*)(?:\s*\(.*\)|\s*[<>=]=?\s*[0-9][0-9.]*\+?)?$").expect("static regex")
    });
    let mut pieces = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in requires.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                pieces.push(&requires[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&requires[start..]);
    let mut out = Vec::new();
    for p in pieces {
        if let Some(c) = PIECE.captures(p.trim()) {
            let bin = match &c[1] {
                "libreoffice" => "soffice",
                "obs-studio" => "obs",
                other => other,
            };
            if !out.iter().any(|b| b == bin) {
                out.push(bin.to_owned());
            }
        }
    }
    out
}

fn sha256_file(path: &Path) -> AaResult<String> {
    Ok(digest::sha256_hex(&std::fs::read(path)?))
}

/// Pins a file. For the interpreter the venv entry is kept (`<realpath of bin/>/python`, not the base
/// interpreter it symlinks to), because a venv is only active when Python starts from inside it; the hash is
/// of the file it resolves to.
fn file_identity(role: &str, path: &Path, keep_last_link: bool) -> AaResult<pb::FileIdentity> {
    let realpath = if keep_last_link {
        let parent = path.parent().unwrap_or(Path::new("/")).canonicalize()?;
        parent.join(path.file_name().unwrap_or_default())
    } else {
        path.canonicalize()?
    };
    Ok(pb::FileIdentity {
        role: role.to_owned(),
        realpath: realpath.display().to_string(),
        sha256: sha256_file(path)?,
        version_banner: String::new(),
    })
}

static ENTRY_DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"["']?([A-Za-z0-9_.-]+)["']?\s*=\s*["']?([A-Za-z_][\w.]*)\s*:\s*([A-Za-z_][\w.]*)"#).expect("static")
});

/// `module:attr` of the console script, from the harness packaging files (setup.py, pyproject.toml, setup.cfg,
/// *.egg-info/entry_points.txt). Text search only; nothing is imported or executed.
fn entry_module(harness: &Path, entry_point: &str) -> Option<String> {
    let mut files = vec![
        harness.join("setup.py"),
        harness.join("pyproject.toml"),
        harness.join("setup.cfg"),
    ];
    if let Ok(rd) = std::fs::read_dir(harness) {
        let mut eggs: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".egg-info"))
            .map(|e| e.path().join("entry_points.txt"))
            .collect();
        eggs.sort();
        files.extend(eggs);
    }
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for c in ENTRY_DECL.captures_iter(&text) {
            let name = c[1].trim_matches(|ch| ch == '"' || ch == '\'');
            if name == entry_point {
                return Some(c[2].to_owned());
            }
        }
    }
    None
}

/// The module file that runs: the venv's installed copy if present, else the checkout's.
fn entry_module_file(harness: &Path, python: &Path, module: &str) -> Option<PathBuf> {
    let rel = module.replace('.', "/");
    let rels = [format!("{rel}.py"), format!("{rel}/__init__.py")];
    if let Some(venv) = python.parent().and_then(Path::parent)
        && let Ok(rd) = std::fs::read_dir(venv.join("lib"))
    {
        let mut libs: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("python"))
            .map(|e| e.path().join("site-packages"))
            .collect();
        libs.sort();
        for lib in libs {
            for r in &rels {
                if lib.join(r).is_file() {
                    return Some(lib.join(r));
                }
            }
        }
    }
    rels.iter().map(|r| harness.join(r)).find(|p| p.is_file())
}

fn detect_license(harness: &Path, repo: &Path) -> pb::License {
    for (dir, label) in [(harness, "harness"), (repo, "repository")] {
        for name in ["LICENSE", "LICENSE.txt", "LICENSE.md", "COPYING"] {
            let Ok(text) = std::fs::read_to_string(dir.join(name)) else {
                continue;
            };
            let spdx = if text.contains("Apache License") && text.contains("Version 2.0") {
                "Apache-2.0"
            } else if text.contains("MIT License") || text.contains("Permission is hereby granted, free of charge") {
                "MIT"
            } else if text.contains("GNU GENERAL PUBLIC LICENSE") && text.contains("Version 3") {
                "GPL-3.0-only"
            } else if text.contains("GNU GENERAL PUBLIC LICENSE") && text.contains("Version 2") {
                "GPL-2.0-only"
            } else if text.contains("BSD") {
                "BSD-3-Clause"
            } else {
                ""
            };
            return pb::License {
                spdx: spdx.to_owned(),
                url: String::new(),
                bring_your_own: false,
                notes: format!("detected from the {label} {name}"),
            };
        }
    }
    pb::License {
        notes: "no LICENSE file found in the harness or repository".into(),
        ..Default::default()
    }
}

/// The STATIC raw artifacts for one registry entry (registry entry, install_cmd, SKILL.md, tree manifest).
fn static_raws(repo: &Path, entry: &RegistryEntry, value: &Value, harness: &Path) -> AaResult<Vec<RawHarvest>> {
    let reg = registry_path(repo).display().to_string();
    let stat = pb::DiscoveryAuthority::Static;
    let mut raws = vec![common::raw(
        HARVESTER,
        RAW_REGISTRY_ENTRY,
        "application/json",
        format!("file:{reg}#clis[name={}]", entry.name),
        stat,
        digest::jcs(value)?,
    )];
    if let Some(cmd) = &entry.install_cmd {
        raws.push(common::raw(
            HARVESTER,
            RAW_INSTALL_CMD,
            "text/plain; charset=utf-8",
            format!("file:{reg}#clis[name={}].install_cmd", entry.name),
            stat,
            cmd.as_bytes().to_vec(),
        ));
    }
    if let Some(rel) = entry.skill_md.as_deref().filter(|s| !s.contains("://")) {
        let repo_real = repo.canonicalize()?;
        if let Ok(path) = repo.join(rel).canonicalize()
            && path.starts_with(&repo_real)
            && path.is_file()
        {
            raws.push(common::raw(
                HARVESTER,
                RAW_SKILL_MD,
                "text/markdown; charset=utf-8",
                format!("file:{}", path.display()),
                stat,
                std::fs::read(&path)?,
            ));
        }
    }
    raws.push(common::raw(
        HARVESTER,
        RAW_TREE_MANIFEST,
        "text/plain; charset=utf-8",
        format!("file:{}", harness.display()),
        stat,
        tree_manifest(harness)?,
    ));
    Ok(raws)
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Starter overlay (`aa.overlay/v1`): click-info harvest, engine-supplied `--project`, `repl` excluded.
fn starter_overlay(target: &pb::Target, entry: &RegistryEntry, authorization: &str) -> String {
    let repl = ids::capability_id(&target.id, &["repl"]);
    let mut y = String::new();
    y.push_str(
        "# Starter overlay written by `aa import cli-anything` (STATIC import; nothing was installed or run).\n",
    );
    y.push_str("schema: aa.overlay/v1\n");
    y.push_str(&format!("target: {}\n", target.id));
    y.push_str(&format!("display_name: {}\n", yaml_str(&target.display_name)));
    y.push_str(&format!("mcp_prefix: {}\n", yaml_str(&target.mcp_prefix)));
    y.push_str(&format!("authorization: {}\n", yaml_str(authorization)));
    y.push_str("harvest:\n");
    y.push_str("  - harvester: click-info@1\n");
    y.push_str(&format!(
        "    options: {{ entry_point: {} }}\n",
        yaml_str(&entry.entry_point)
    ));
    y.push_str("defaults:\n  binding:\n    deadline_ms: 30000\n    usage_exit_code: 2\n");
    y.push_str(
        "# CLI-Anything --project: engine-supplied from the session workspace (never agent-chosen by default).\n",
    );
    y.push_str("state_handle: { flag: \"--project\", engine_supplied: true }\n");
    y.push_str("exclude:\n");
    y.push_str(&format!(
        "  - {{ capability: {repl}, reason: \"interactive REPL (TTY-only)\" }}\n"
    ));
    y
}

/// STATIC only: reads files; hashes the harness tree (git tree sha if .git present, else sorted blake3 manifest).
pub fn import_harness(
    repo: &std::path::Path,
    name: &str,
    python: &std::path::Path,
    authorization: &str,
) -> AaResult<ImportedHarness> {
    if authorization.trim().is_empty() {
        return Err(AaError::invalid(
            "an authorization statement is required to import a harness (it permits later introspection)",
        ));
    }
    let (entry, value) = find_entry(repo, name)?;
    let target_id = target_id_for(&entry.name)?;
    let harness = harness_dir(repo, &entry)?;
    if !python.is_absolute() || !python.is_file() {
        return Err(AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!(
                "interpreter {} does not exist (the engine never installs harnesses)",
                python.display()
            ),
            format!(
                "install it first (operator step): uv venv <dir> && uv pip install --python <dir>/bin/python {}; \
                 then: aa import cli-anything <repo> --harness {} --python <dir>/bin/python",
                harness.display(),
                entry.name
            ),
        ));
    }
    let mut files = vec![file_identity("interpreter", python, true)?];
    if let Some(app) = common::framework_python_app(python) {
        files.push(file_identity("interpreter:app", &app, false)?);
    }
    let module = entry_module(&harness, &entry.entry_point).ok_or_else(|| {
        AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!(
                "no console-script declaration for '{}' in {}",
                entry.entry_point,
                harness.display()
            ),
            "check the harness packaging (setup.py / pyproject.toml), then: aa import cli-anything <repo> …",
        )
    })?;
    let module_file = entry_module_file(&harness, python, &module).ok_or_else(|| {
        AaError::new(
            pb::ErrorCode::ConfigurationInvalid,
            format!("entry module {module} not found in the venv or the checkout"),
            "install the harness into the interpreter's venv (operator step), then: aa import cli-anything <repo> …",
        )
    })?;
    files.push(file_identity("entry_module", &module_file, false)?);
    let mut preconditions = Vec::new();
    if let Some(requires) = entry.requires.as_deref().filter(|r| !r.trim().is_empty()) {
        let text = common::target_text(requires, pb::ContentOrigin::Target, "registry.json:requires");
        let bins = required_binaries(requires);
        for bin in &bins {
            preconditions.push(pb::Precondition {
                id: format!("binary:{bin}"),
                kind: pb::PreconditionKind::Binary as i32,
                description: Some(text.clone()),
                needs_human: false,
                check_capability_id: String::new(),
                confidence: pb::Confidence::Inferred as i32,
            });
            // Pinned when it resolves now; otherwise the precondition stays unmet (no silent fallback).
            if let Ok(found) = which::which(bin)
                && let Ok(fi) = file_identity(&format!("backend:{bin}"), &found, false)
            {
                files.push(fi);
            }
        }
        if bins.is_empty() {
            preconditions.push(pb::Precondition {
                id: "context:registry-requires".into(),
                kind: pb::PreconditionKind::Context as i32,
                description: Some(text),
                needs_human: true,
                check_capability_id: String::new(),
                confidence: pb::Confidence::Inferred as i32,
            });
        }
    }
    let raw = static_raws(repo, &entry, &value, &harness)?;
    let manifest = raw
        .iter()
        .find(|r| r.name == RAW_TREE_MANIFEST)
        .map(|r| r.bytes.as_slice())
        .unwrap_or_default();
    let mut identity = pb::TargetIdentity {
        files,
        source_tree_sha: source_tree_sha(manifest),
        ..Default::default()
    };
    identity.identity_digest = digest::self_digest(&identity, |i| i.identity_digest.clear())?;
    let sources = raw
        .iter()
        .map(|r| pb::SourceRef {
            harvester: r.harvester.clone(),
            uri: r.uri.clone(),
            sha256: r.sha256(),
            authority: r.authority,
            harvested_at: None,
            action_id: String::new(),
        })
        .collect();
    let target = pb::Target {
        id: target_id,
        display_name: entry.display_name.clone(),
        kind: pb::TargetKind::CliAnythingHarness as i32,
        identity: Some(identity),
        license: Some(detect_license(&harness, repo)),
        preconditions,
        sources,
        status: pb::TargetStatus::Scaffolded as i32,
        authorization_statement: authorization.to_owned(),
        fixtures: Vec::new(),
        mcp_prefix: entry
            .name
            .to_ascii_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
        description: Some(common::target_text(
            entry.description.clone(),
            pb::ContentOrigin::Target,
            "registry.json:description",
        )),
        families: Vec::new(),
    };
    let overlay_yaml = starter_overlay(&target, &entry, authorization);
    let install_cmd_flags = entry.install_cmd.as_deref().map(install_cmd_flags).unwrap_or_default();
    Ok(ImportedHarness {
        target,
        overlay_yaml,
        raw,
        install_cmd_flags,
    })
}

/// id "cli-anything-registry", v1, STATIC.
pub struct CliAnythingRegistryHarvester;

#[async_trait::async_trait]
impl Harvester for CliAnythingRegistryHarvester {
    fn id(&self) -> &'static str {
        "cli-anything-registry"
    }

    fn version(&self) -> u32 {
        1
    }

    fn authority(&self) -> pb::DiscoveryAuthority {
        pb::DiscoveryAuthority::Static
    }

    /// Needs `options.repo` (the local checkout); `options.harness` defaults to the target's registry name.
    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool {
        target.kind == pb::TargetKind::CliAnythingHarness as i32 && common::opt_str(options, "repo").is_some()
    }

    /// STATIC: re-reads the registry entry, install_cmd, SKILL.md and tree manifest. Never runs anything.
    async fn harvest(&self, ctx: HarvestCtx<'_>) -> AaResult<Vec<RawHarvest>> {
        let repo = common::opt_str(ctx.options, "repo")
            .map(PathBuf::from)
            .ok_or_else(|| AaError::invalid(format!("{HARVESTER} needs harvest[].options.repo")))?;
        let name = match common::opt_str(ctx.options, "harness") {
            Some(n) => n.to_owned(),
            None => ctx
                .target
                .id
                .strip_prefix("cli-anything.")
                .map(str::to_owned)
                .ok_or_else(|| AaError::invalid(format!("{HARVESTER} needs harvest[].options.harness")))?,
        };
        let (entry, value) = find_entry(&repo, &name)?;
        let harness = harness_dir(&repo, &entry)?;
        static_raws(&repo, &entry, &value, &harness)
    }

    /// The registry carries no command or effect metadata: no candidates (click-info@1 provides them).
    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> AaResult<Vec<Candidate>> {
        let _ = (target, raw);
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_cmd_review() {
        let lo = "pip install git+https://github.com/HKUDS/CLI-Anything.git#subdirectory=libreoffice/agent-harness";
        assert_eq!(install_cmd_flags(lo), ["unpinned-git-url"]);
        let pinned = "pip install git+https://github.com/a/b.git@0123456789abcdef0123456789abcdef01234567";
        assert!(install_cmd_flags(pinned).is_empty());
        assert_eq!(
            install_cmd_flags("pip install git+https://x/y.git@v1"),
            ["mutable-git-ref"]
        );
        assert_eq!(
            install_cmd_flags("pip install cli-anything-zotero"),
            ["unpinned-package"]
        );
        assert!(install_cmd_flags("pip install cli-anything-zotero==1.2").is_empty());
        let npm = install_cmd_flags("cd sketch/agent-harness && npm install && npm link");
        assert_eq!(npm, ["not-pip", "shell-metachar"]);
        let curl = install_cmd_flags("curl -fsSL https://x/install.sh | bash");
        assert!(curl.contains(&"remote-script".to_owned()) && curl.contains(&"shell-metachar".to_owned()));
    }

    #[test]
    fn requires_to_binaries() {
        assert_eq!(required_binaries("libreoffice"), ["soffice"]);
        assert_eq!(required_binaries("gimp (apt install gimp)"), ["gimp"]);
        assert_eq!(required_binaries("blender >= 4.2"), ["blender"]);
        assert_eq!(
            required_binaries("melt (apt install melt), ffmpeg (apt install ffmpeg)"),
            ["melt", "ffmpeg"]
        );
        assert!(required_binaries("ComfyUI running at http://localhost:8188").is_empty());
        assert!(required_binaries("Calibre (apt install calibre, brew install --cask calibre)").is_empty());
        assert!(required_binaries("ANYGEN_API_KEY").is_empty());
    }

    #[test]
    fn target_ids_are_valid() {
        assert_eq!(target_id_for("libreoffice").unwrap(), "cli-anything.libreoffice");
        assert_eq!(
            target_id_for("slay_the_spire_ii").unwrap(),
            "cli-anything.slay-the-spire-ii"
        );
        assert_eq!(target_id_for("3MF").unwrap(), "cli-anything.3mf");
    }

    #[test]
    fn manifest_excludes_build_products_and_is_order_independent() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("pkg/__pycache__")).unwrap();
        std::fs::create_dir_all(d.path().join("build/lib")).unwrap();
        std::fs::create_dir_all(d.path().join("x.egg-info")).unwrap();
        std::fs::write(d.path().join("pkg/b.py"), "b").unwrap();
        std::fs::write(d.path().join("pkg/a.py"), "a").unwrap();
        let before = tree_manifest(d.path()).unwrap();
        std::fs::write(d.path().join("pkg/__pycache__/a.cpython-314.pyc"), "x").unwrap();
        std::fs::write(d.path().join("build/lib/a.py"), "a").unwrap();
        std::fs::write(d.path().join("x.egg-info/PKG-INFO"), "p").unwrap();
        assert_eq!(tree_manifest(d.path()).unwrap(), before);
        let text = String::from_utf8(before.clone()).unwrap();
        assert!(
            text.lines().next().unwrap().ends_with(" pkg/a.py"),
            "sorted by path: {text}"
        );
        assert_eq!(text.lines().count(), 2);
        std::fs::write(d.path().join("pkg/a.py"), "changed").unwrap();
        assert_ne!(
            source_tree_sha(&tree_manifest(d.path()).unwrap()),
            source_tree_sha(&before)
        );
    }
}
