//! Hash-chained JSONL audit log (`audit/segment-<n>.jsonl` + `audit/HEAD`), intent recovery at boot (ADR 0006).
//!
//! - Each line is one [`pb::AuditEntry`] in canonical proto3-JSON (JCS), terminated by `\n`.
//!   `entry_sha256 = sha256(JCS(entry with entry_sha256 = ""))`; `prev_sha256` is the previous entry's
//!   `entry_sha256` (`""` for seq 1); seq starts at 1 and is contiguous across segments.
//! - `audit/HEAD` is `"<seq> <entry_sha256>\n"` of the last entry, replaced by atomic rename after every append,
//!   so truncation of the tail is visible at the next boot.
//! - A single writer: the log holds an exclusive `flock` on `audit/LOCK` for its lifetime, and every append
//!   runs under one `Mutex`. Segments roll over at 64 MiB.
//! - Verification is strict: a line must parse, be byte-identical to the JCS of what it parses to, carry the
//!   positional seq, link to its predecessor and match its own digest. The log is never truncated or
//!   rewritten; after a torn final line (crash mid-write) appends continue in a fresh segment.
//!
//! `AuditLog::open` accepts either the audit directory itself (a path whose last component is `audit`) or the
//! workspace root (then `<root>/audit` is used).

use crate::fsutil::{self, io_err};
use aa_core::{AaError, AaResult, api::AuditSink, digest, pb};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

/// Segment rollover threshold.
pub const SEGMENT_MAX_BYTES: u64 = 64 * 1024 * 1024;
const HEAD: &str = "HEAD";
const LOCK: &str = "LOCK";
const SEGMENT_PREFIX: &str = "segment-";
const SEGMENT_SUFFIX: &str = ".jsonl";

/// Single writer behind a Mutex; segment rollover at 64 MiB.
pub struct AuditLog {
    inner: Mutex<Inner>,
}

#[derive(Debug, Clone)]
pub struct BootReport {
    /// Every line verified **and** `HEAD` matches the last entry. The engine boots halted when this is false.
    pub chain_ok: bool,
    /// Lines present in the log (before any recovery appends made by `open`).
    pub entries: u64,
    /// Positional seq of the first line that fails to parse, is not canonical, or breaks the chain.
    pub first_broken_seq: Option<u64>,
    /// `audit/HEAD` names the last entry (or the one before it: crash between append and HEAD update).
    /// An empty log with no HEAD matches.
    pub head_matches: bool,
    /// Action ids closed at boot by a recovered `ActionRecord` (only when `chain_ok`; otherwise deferred).
    pub recovered_action_ids: Vec<String>,
}

/// Filters for [`AuditLog::query`]. All set filters must match. `limit == 0` means no limit.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Entries whose capability id (intent/action) or admin subject is the target id or starts with `<id>.`.
    pub target_id: Option<String>,
    /// Exact capability id (intent/action) or admin subject.
    pub capability_id: Option<String>,
    /// Exact action id (intent/action).
    pub action_id: Option<String>,
    /// Only entries with `seq > after_seq`.
    pub after_seq: u64,
    pub limit: u32,
    /// Only ActionRecords (skip intents and admin events).
    pub actions_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct Loc {
    segment: u32,
    offset: u64,
    len: u64,
}

struct Writer {
    segment: u32,
    file: File,
    len: u64,
}

struct Inner {
    dir: PathBuf,
    _lock: File,
    segment_limit: u64,
    /// Highest segment number present (0 = none).
    last_segment: u32,
    writer: Option<Writer>,
    /// The next append must start a new segment (torn tail at boot, or a failed write).
    force_new_segment: bool,
    head_seq: u64,
    head_sha: String,
    /// `locs[i]` is the line at positional seq `i + 1`.
    locs: Vec<Loc>,
    action_by_id: HashMap<String, u64>,
    action_by_idempotency_key: HashMap<String, u64>,
}

/// One pass over the segments on disk.
struct Scan {
    lines: u64,
    first_broken: Option<u64>,
    /// The last line's recorded `entry_sha256` (or the sha256 of its raw bytes if it did not parse).
    last_sha: String,
    /// The same for the line before it (`""` if fewer than two lines).
    prev_last_sha: String,
    last_segment: u32,
    torn_tail: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn audit_unavailable(message: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::AuditUnavailable,
        message,
        "check the audit volume (space, permissions), then aa audit --verify",
    )
}

fn now() -> pbjson_types::Timestamp {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    pbjson_types::Timestamp {
        seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(d.subsec_nanos()).unwrap_or(0),
    }
}

fn segment_name(n: u32) -> String {
    format!("{SEGMENT_PREFIX}{n:06}{SEGMENT_SUFFIX}")
}

/// Segment numbers present in `dir`, ascending.
fn list_segments(dir: &Path) -> AaResult<Vec<u32>> {
    let mut out = Vec::new();
    for ent in fs::read_dir(dir).map_err(|e| io_err(format_args!("cannot list {}", dir.display()), &e))? {
        let ent = ent.map_err(|e| io_err(format_args!("cannot list {}", dir.display()), &e))?;
        let name = ent.file_name();
        let Some(num) = name
            .to_str()
            .and_then(|n| n.strip_prefix(SEGMENT_PREFIX))
            .and_then(|n| n.strip_suffix(SEGMENT_SUFFIX))
        else {
            continue;
        };
        if num.len() >= 6
            && num.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = num.parse::<u32>()
            && n > 0
        {
            out.push(n);
        }
    }
    out.sort_unstable();
    Ok(out)
}

fn entry_digest(e: &pb::AuditEntry) -> AaResult<String> {
    digest::self_digest(e, |e| e.entry_sha256.clear())
}

/// Parses and fully checks one line (without its `\n`). `Ok(entry)` only if the line is canonical, carries
/// `expected_seq`, links to `prev` and matches its own digest; `Err(Some(entry))` if it parsed but failed a check.
fn check_line(line: &[u8], expected_seq: u64, prev: &str) -> Result<pb::AuditEntry, Option<pb::AuditEntry>> {
    let entry: pb::AuditEntry = serde_json::from_slice(line).map_err(|_| None)?;
    let canonical = digest::jcs(&entry).is_ok_and(|c| c == line);
    let digest_ok = entry_digest(&entry).is_ok_and(|d| d == entry.entry_sha256);
    if canonical && digest_ok && entry.seq == expected_seq && entry.prev_sha256 == prev {
        Ok(entry)
    } else {
        Err(Some(entry))
    }
}

/// Reads every segment in order, calling `visit(seq, loc, entry)` for each line (entry is `None` when the line
/// does not parse). Verification results are returned in the [`Scan`].
fn scan(dir: &Path, mut visit: impl FnMut(u64, Loc, Option<&pb::AuditEntry>)) -> AaResult<Scan> {
    let segments = list_segments(dir)?;
    let mut s = Scan {
        lines: 0,
        first_broken: None,
        last_sha: String::new(),
        prev_last_sha: String::new(),
        last_segment: segments.last().copied().unwrap_or(0),
        torn_tail: false,
    };
    let mut buf = Vec::new();
    for (i, &seg) in segments.iter().enumerate() {
        let path = dir.join(segment_name(seg));
        let file = File::open(&path).map_err(|e| io_err(format_args!("cannot open {}", path.display()), &e))?;
        let mut reader = BufReader::new(file);
        let mut offset = 0u64;
        loop {
            buf.clear();
            let n = reader
                .read_until(b'\n', &mut buf)
                .map_err(|e| io_err(format_args!("cannot read {}", path.display()), &e))?;
            if n == 0 {
                break;
            }
            s.lines += 1;
            let seq = s.lines;
            let terminated = buf.last() == Some(&b'\n');
            let body = if terminated { &buf[..buf.len() - 1] } else { &buf[..] };
            let loc = Loc {
                segment: seg,
                offset,
                len: body.len() as u64,
            };
            offset += n as u64;
            if !terminated && i + 1 == segments.len() {
                s.torn_tail = true;
            }
            let checked = if terminated {
                check_line(body, seq, &s.last_sha)
            } else {
                Err(serde_json::from_slice(body).ok())
            };
            let parsed = match checked {
                Ok(e) => Some(e),
                Err(e) => {
                    s.first_broken.get_or_insert(seq);
                    e
                }
            };
            s.prev_last_sha = std::mem::take(&mut s.last_sha);
            s.last_sha = parsed
                .as_ref()
                .map_or_else(|| digest::sha256_hex(body), |e| e.entry_sha256.clone());
            visit(seq, loc, parsed.as_ref());
        }
    }
    Ok(s)
}

/// `(seq, sha)` from `audit/HEAD`; `None` if missing or malformed.
fn read_head(dir: &Path) -> Option<(u64, String)> {
    let text = fs::read_to_string(dir.join(HEAD)).ok()?;
    let (seq, sha) = text.trim_end_matches('\n').split_once(' ')?;
    Some((seq.parse().ok()?, sha.to_owned()))
}

/// HEAD names the last entry, or lags it by exactly one entry it correctly names: the window between a line
/// reaching the segment and HEAD's rename, so an engine crash mid-append is not mistaken for truncation.
/// A HEAD *ahead of* the log (a deleted tail), or naming a different entry, does not match.
fn head_matches(dir: &Path, scan: &Scan) -> bool {
    match read_head(dir) {
        Some((seq, sha)) => {
            (seq == scan.lines && sha == scan.last_sha)
                || (scan.lines > 1 && seq == scan.lines - 1 && sha == scan.prev_last_sha)
        }
        None => scan.lines == 0 && !dir.join(HEAD).exists(),
    }
}

fn report(dir: &Path, scan: &Scan) -> BootReport {
    let head_matches = head_matches(dir, scan);
    BootReport {
        chain_ok: scan.first_broken.is_none() && head_matches,
        entries: scan.lines,
        first_broken_seq: scan.first_broken,
        head_matches,
        recovered_action_ids: Vec::new(),
    }
}

fn engine_internal_principal() -> pb::Principal {
    pb::Principal {
        role: pb::Role::EngineInternal as i32,
        channel: pb::Channel::EngineInternal as i32,
        uid: fsutil::euid(),
        pid: i32::try_from(std::process::id()).unwrap_or(0),
        client_name: "aad".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
        ..Default::default()
    }
}

fn recovered_record(intent: &pb::ActionIntent, intent_seq: u64) -> pb::ActionRecord {
    let error = AaError::new(
        pb::ErrorCode::ExecutionStateUnknown,
        format!(
            "the engine stopped after recording the intent for {} (audit seq {intent_seq}) and before its outcome; \
             the action may or may not have happened",
            intent.action_id
        ),
        "inspect the target state before retrying",
    )
    .with_recovery(
        pb::RecoveryKind::CheckStatusBeforeRetry,
        "RuntimeService.GetActionStatus",
    );
    pb::ActionRecord {
        action_id: intent.action_id.clone(),
        capability_id: intent.capability_id.clone(),
        capability_digest: intent.capability_digest.clone(),
        args_commitment: intent.args_commitment.clone(),
        action_digest: intent.action_digest.clone(),
        idempotency_key: intent.idempotency_key.clone(),
        level: intent.level,
        decision: intent.decision.clone(),
        principal: Some(engine_internal_principal()),
        outcome: pb::Outcome::Unknown as i32,
        error: Some(error.to_proto()),
        residual_unknowns: vec!["outcome: the engine stopped between intent and outcome".into()],
        completed_at: Some(now()),
        recovered: true,
        ..Default::default()
    }
}

impl Inner {
    fn read_loc(&self, loc: Loc) -> AaResult<Vec<u8>> {
        let path = self.dir.join(segment_name(loc.segment));
        let mut f = File::open(&path).map_err(|e| io_err(format_args!("cannot open {}", path.display()), &e))?;
        let len = usize::try_from(loc.len).map_err(|_| AaError::internal("audit line too long"))?;
        let mut buf = vec![0u8; len];
        f.seek(SeekFrom::Start(loc.offset))
            .and_then(|_| f.read_exact(&mut buf))
            .map_err(|e| io_err(format_args!("cannot read {}", path.display()), &e))?;
        Ok(buf)
    }

    fn entry(&self, seq: u64) -> AaResult<Option<pb::AuditEntry>> {
        let Some(loc) = seq
            .checked_sub(1)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.locs.get(i))
        else {
            return Ok(None);
        };
        let bytes = self.read_loc(*loc)?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| AaError::internal(format!("audit entry {seq} does not parse: {e}")))
    }

    /// Calls `f` for every parseable entry with seq > `after`, in order, until it returns `false`.
    fn for_each_after(&self, after: u64, mut f: impl FnMut(pb::AuditEntry) -> bool) -> AaResult<()> {
        let start = usize::try_from(after).unwrap_or(usize::MAX);
        let mut current: Option<(u32, BufReader<File>, u64)> = None;
        let mut buf = Vec::new();
        for loc in self.locs.iter().skip(start) {
            if current
                .as_ref()
                .is_none_or(|(seg, _, pos)| *seg != loc.segment || *pos != loc.offset)
            {
                let path = self.dir.join(segment_name(loc.segment));
                let mut file =
                    File::open(&path).map_err(|e| io_err(format_args!("cannot open {}", path.display()), &e))?;
                file.seek(SeekFrom::Start(loc.offset))
                    .map_err(|e| io_err(format_args!("cannot read {}", path.display()), &e))?;
                current = Some((loc.segment, BufReader::new(file), loc.offset));
            }
            let Some((_, reader, pos)) = current.as_mut() else {
                unreachable!("reader was just opened")
            };
            buf.clear();
            let n = reader
                .read_until(b'\n', &mut buf)
                .map_err(|e| io_err("cannot read audit segment", &e))?;
            *pos += n as u64;
            let body = buf.strip_suffix(b"\n").unwrap_or(&buf);
            if let Ok(entry) = serde_json::from_slice::<pb::AuditEntry>(body)
                && !f(entry)
            {
                break;
            }
        }
        Ok(())
    }

    fn open_new_segment(&mut self) -> AaResult<()> {
        let segment = self.last_segment + 1;
        let path = self.dir.join(segment_name(segment));
        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| audit_unavailable(format!("cannot create {}: {e}", path.display())))?;
        fsutil::fsync_dir(&self.dir);
        self.last_segment = segment;
        self.writer = Some(Writer { segment, file, len: 0 });
        self.force_new_segment = false;
        Ok(())
    }

    fn write_head(&self, durable: bool) -> AaResult<()> {
        let tmp = self.dir.join("HEAD.tmp");
        let res = (|| -> io::Result<()> {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(format!("{} {}\n", self.head_seq, self.head_sha).as_bytes())?;
            if durable {
                f.sync_all()?;
            }
            fs::rename(&tmp, self.dir.join(HEAD))
        })();
        res.map_err(|e| audit_unavailable(format!("cannot update audit HEAD: {e}")))?;
        if durable {
            fsutil::fsync_dir(&self.dir);
        }
        Ok(())
    }

    fn append(
        &mut self,
        principal: &pb::Principal,
        halt_epoch: u64,
        body: pb::audit_entry::Body,
        durable: bool,
    ) -> AaResult<u64> {
        let seq = self.head_seq + 1;
        let mut entry = pb::AuditEntry {
            seq,
            prev_sha256: self.head_sha.clone(),
            entry_sha256: String::new(),
            written_at: Some(now()),
            principal: Some(principal.clone()),
            halt_epoch,
            body: Some(body),
        };
        entry.entry_sha256 = entry_digest(&entry)?;
        let mut line = digest::jcs(&entry)?;
        line.push(b'\n');

        let needs_new = match &self.writer {
            None => true,
            Some(w) => self.force_new_segment || (w.len > 0 && w.len + line.len() as u64 > self.segment_limit),
        };
        if needs_new {
            self.open_new_segment()?;
        }
        let Some(w) = self.writer.as_mut() else {
            unreachable!("a segment was just opened")
        };
        if let Err(e) = w.file.write_all(&line) {
            // A partial line may be on disk: never append after it.
            self.force_new_segment = true;
            return Err(audit_unavailable(format!("cannot append audit entry {seq}: {e}")));
        }
        let loc = Loc {
            segment: w.segment,
            offset: w.len,
            len: line.len() as u64 - 1,
        };
        w.len += line.len() as u64;
        // The line is in the file from here on: account for it even if fsync fails.
        let synced = if durable { w.file.sync_data() } else { Ok(()) };
        self.locs.push(loc);
        self.head_seq = seq;
        self.head_sha = entry.entry_sha256.clone();
        if let Some(pb::audit_entry::Body::Action(rec)) = &entry.body {
            self.index_action(rec, seq);
        }
        synced.map_err(|e| audit_unavailable(format!("cannot fsync audit entry {seq}: {e}")))?;
        self.write_head(durable)?;
        Ok(seq)
    }

    fn index_action(&mut self, rec: &pb::ActionRecord, seq: u64) {
        self.action_by_id.insert(rec.action_id.clone(), seq);
        if !rec.idempotency_key.is_empty() {
            self.action_by_idempotency_key.insert(rec.idempotency_key.clone(), seq);
        }
    }

    fn action_at(&self, seq: Option<u64>) -> AaResult<Option<pb::ActionRecord>> {
        let Some(seq) = seq else { return Ok(None) };
        Ok(self.entry(seq)?.and_then(|e| match e.body {
            Some(pb::audit_entry::Body::Action(a)) => Some(a),
            _ => None,
        }))
    }
}

impl AuditLog {
    /// Verifies the chain + HEAD, then closes dangling intents with recovered ActionRecords (OUTCOME_UNKNOWN,
    /// EXECUTION_STATE_UNKNOWN, recovered=true, principal ENGINE_INTERNAL). Never truncates or rewrites.
    pub fn open(dir: &std::path::Path) -> AaResult<(Self, BootReport)> {
        Self::open_with_limit(dir, SEGMENT_MAX_BYTES)
    }

    fn open_with_limit(dir: &Path, segment_limit: u64) -> AaResult<(Self, BootReport)> {
        let dir = fsutil::resolve_subdir(dir, "audit");
        fsutil::ensure_private_dir(&dir, fsutil::ANY_GROUP_OTHER)?;
        let lock_path = dir.join(LOCK);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|e| audit_unavailable(format!("cannot open {}: {e}", lock_path.display())))?;
        rustix::fs::flock(&lock_file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            audit_unavailable(format!(
                "the audit log {} is held by another writer ({e}); is another aad running?",
                dir.display()
            ))
        })?;

        let mut locs = Vec::new();
        let mut action_by_id = HashMap::new();
        let mut action_by_idempotency_key = HashMap::new();
        // Intents not (yet) followed by an ActionRecord with the same action id: (intent seq, epoch, intent).
        let mut pending: HashMap<String, (u64, u64, pb::ActionIntent)> = HashMap::new();
        let scanned = scan(&dir, |seq, loc, entry| {
            locs.push(loc);
            match entry.and_then(|e| e.body.as_ref().map(|b| (e.halt_epoch, b))) {
                Some((_, pb::audit_entry::Body::Action(rec))) => {
                    pending.remove(&rec.action_id);
                    action_by_id.insert(rec.action_id.clone(), seq);
                    if !rec.idempotency_key.is_empty() {
                        action_by_idempotency_key.insert(rec.idempotency_key.clone(), seq);
                    }
                }
                Some((epoch, pb::audit_entry::Body::Intent(intent)))
                    if !action_by_id.contains_key(&intent.action_id) =>
                {
                    pending.insert(intent.action_id.clone(), (seq, epoch, intent.clone()));
                }
                _ => {}
            }
        })?;
        let mut boot = report(&dir, &scanned);
        if !boot.chain_ok {
            tracing::error!(
                dir = %dir.display(),
                first_broken_seq = ?boot.first_broken_seq,
                head_matches = boot.head_matches,
                "audit chain failed to verify"
            );
        }

        let last_segment = scanned.last_segment;
        let mut inner = Inner {
            dir: dir.clone(),
            _lock: lock_file,
            segment_limit,
            last_segment,
            writer: None,
            force_new_segment: scanned.torn_tail,
            head_seq: scanned.lines,
            head_sha: scanned.last_sha.clone(),
            locs,
            action_by_id,
            action_by_idempotency_key,
        };
        if last_segment > 0 && !scanned.torn_tail {
            let path = dir.join(segment_name(last_segment));
            let file = OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| audit_unavailable(format!("cannot open {}: {e}", path.display())))?;
            let len = file
                .metadata()
                .map_err(|e| audit_unavailable(format!("cannot stat {}: {e}", path.display())))?
                .len();
            inner.writer = Some(Writer {
                segment: last_segment,
                file,
                len,
            });
        }

        let mut dangling: Vec<_> = pending.into_values().collect();
        dangling.sort_by_key(|(seq, _, _)| *seq);
        if !boot.chain_ok && !dangling.is_empty() {
            // Never synthesize records from a log that failed to verify: a corrupted line may be the very
            // ActionRecord that closes an intent. Recovery runs at the first boot whose chain verifies.
            tracing::warn!(
                dangling = dangling.len(),
                "audit chain did not verify; intent recovery deferred"
            );
            dangling.clear();
        }
        let principal = engine_internal_principal();
        for (seq, epoch, intent) in dangling {
            let rec = recovered_record(&intent, seq);
            inner.append(&principal, epoch, pb::audit_entry::Body::Action(rec), true)?;
            tracing::warn!(action_id = %intent.action_id, intent_seq = seq, "closed dangling intent as OUTCOME_UNKNOWN");
            boot.recovered_action_ids.push(intent.action_id);
        }

        Ok((
            Self {
                inner: Mutex::new(inner),
            },
            boot,
        ))
    }

    /// Entries matching `q`, in seq order.
    pub fn query(&self, q: &AuditQuery) -> AaResult<Vec<pb::AuditEntry>> {
        let inner = lock(&self.inner);
        let limit = if q.limit == 0 { usize::MAX } else { q.limit as usize };
        let mut out = Vec::new();
        inner.for_each_after(q.after_seq, |e| {
            if matches(q, &e) {
                out.push(e);
            }
            out.len() < limit
        })?;
        Ok(out)
    }

    /// Re-reads and re-verifies the whole log from disk now (`recovered_action_ids` is empty).
    pub fn verify_chain(&self) -> BootReport {
        let inner = lock(&self.inner);
        match scan(&inner.dir, |_, _, _| {}) {
            Ok(s) => report(&inner.dir, &s),
            Err(e) => {
                tracing::error!(error = %e, "audit chain could not be read");
                BootReport {
                    chain_ok: false,
                    entries: inner.head_seq,
                    first_broken_seq: Some(1),
                    head_matches: false,
                    recovered_action_ids: Vec::new(),
                }
            }
        }
    }

    /// The ActionRecord for `action_id` (the latest, should there ever be more than one).
    pub fn find_action(&self, action_id: &str) -> AaResult<Option<pb::ActionRecord>> {
        let inner = lock(&self.inner);
        inner.action_at(inner.action_by_id.get(action_id).copied())
    }

    /// The most recent ActionRecord carrying this (non-empty) idempotency key.
    pub fn find_by_idempotency_key(&self, key: &str) -> AaResult<Option<pb::ActionRecord>> {
        if key.is_empty() {
            return Ok(None);
        }
        let inner = lock(&self.inner);
        inner.action_at(inner.action_by_idempotency_key.get(key).copied())
    }
}

fn matches(q: &AuditQuery, e: &pb::AuditEntry) -> bool {
    use pb::audit_entry::Body;
    let (action_id, subject) = match &e.body {
        Some(Body::Action(a)) => (Some(a.action_id.as_str()), a.capability_id.as_str()),
        Some(Body::Intent(i)) if !q.actions_only => (Some(i.action_id.as_str()), i.capability_id.as_str()),
        Some(Body::Admin(a)) if !q.actions_only => (None, a.subject.as_str()),
        _ => return false,
    };
    if q.action_id.as_deref().is_some_and(|want| action_id != Some(want)) {
        return false;
    }
    if q.capability_id.as_deref().is_some_and(|want| subject != want) {
        return false;
    }
    if let Some(t) = q.target_id.as_deref()
        && !subject
            .strip_prefix(t)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    {
        return false;
    }
    true
}

/// append_* = assign seq, prev, entry_sha256 = sha256(JCS(entry with "")), write line, fsync (always for
/// intent/admin; action: fsync if level >= WRITE), atomic-rename HEAD.
///
/// Actions whose level is `EFFECT_LEVEL_UNKNOWN` (or unrecognized) are fsynced too (fail-safe).
impl AuditSink for AuditLog {
    fn append_intent(&self, principal: &pb::Principal, halt_epoch: u64, intent: pb::ActionIntent) -> AaResult<u64> {
        lock(&self.inner).append(principal, halt_epoch, pb::audit_entry::Body::Intent(intent), true)
    }

    fn append_action(&self, principal: &pb::Principal, halt_epoch: u64, record: pb::ActionRecord) -> AaResult<u64> {
        let durable = !matches!(
            pb::EffectLevel::try_from(record.level),
            Ok(pb::EffectLevel::Read | pb::EffectLevel::Ui)
        );
        lock(&self.inner).append(principal, halt_epoch, pb::audit_entry::Body::Action(record), durable)
    }

    fn append_admin(&self, principal: &pb::Principal, halt_epoch: u64, event: pb::AdminEvent) -> AaResult<u64> {
        lock(&self.inner).append(principal, halt_epoch, pb::audit_entry::Body::Admin(event), true)
    }

    fn head(&self) -> (u64, String) {
        let inner = lock(&self.inner);
        (inner.head_seq, inner.head_sha.clone())
    }

    fn entry(&self, seq: u64) -> AaResult<Option<pb::AuditEntry>> {
        lock(&self.inner).entry(seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal() -> pb::Principal {
        pb::Principal {
            role: pb::Role::Agent as i32,
            channel: pb::Channel::AgentSocket as i32,
            uid: 501,
            ..Default::default()
        }
    }

    fn read_action(id: &str, cap: &str) -> pb::ActionRecord {
        pb::ActionRecord {
            action_id: id.into(),
            capability_id: cap.into(),
            level: pb::EffectLevel::Read as i32,
            outcome: pb::Outcome::Completed as i32,
            ..Default::default()
        }
    }

    #[test]
    fn segments_roll_over_and_chain_spans_them() {
        let d = tempfile::tempdir().unwrap();
        {
            let (log, boot) = AuditLog::open_with_limit(d.path(), 600).unwrap();
            assert!(boot.chain_ok && boot.entries == 0);
            for i in 0..20 {
                log.append_action(&principal(), 0, read_action(&format!("act_{i}"), "git.status"))
                    .unwrap();
            }
            assert_eq!(log.head().0, 20);
        }
        let segs = list_segments(&d.path().join("audit")).unwrap();
        assert!(segs.len() > 2, "expected rollover, got {segs:?}");
        let (log, boot) = AuditLog::open_with_limit(d.path(), 600).unwrap();
        assert!(boot.chain_ok, "{boot:?}");
        assert_eq!(boot.entries, 20);
        for seq in 1..=20 {
            assert_eq!(log.entry(seq).unwrap().unwrap().seq, seq);
        }
        let q = log
            .query(&AuditQuery {
                after_seq: 5,
                limit: 3,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(q.iter().map(|e| e.seq).collect::<Vec<_>>(), [6, 7, 8]);
        assert_eq!(log.query(&AuditQuery::default()).unwrap().len(), 20);
    }

    #[test]
    fn second_writer_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let (_log, _) = AuditLog::open(d.path()).unwrap();
        assert_eq!(
            AuditLog::open(d.path()).err().unwrap().code,
            pb::ErrorCode::AuditUnavailable
        );
    }

    #[test]
    fn torn_tail_is_reported_and_appends_go_to_a_new_segment() {
        let d = tempfile::tempdir().unwrap();
        {
            let (log, _) = AuditLog::open(d.path()).unwrap();
            log.append_action(&principal(), 0, read_action("act_1", "git.status"))
                .unwrap();
        }
        let seg1 = d.path().join("audit").join(segment_name(1));
        let mut f = OpenOptions::new().append(true).open(&seg1).unwrap();
        f.write_all(b"{\"seq\":\"2\",\"prev").unwrap();
        drop(f);
        let before = fs::read(&seg1).unwrap();
        let (log, boot) = AuditLog::open(d.path()).unwrap();
        assert!(!boot.chain_ok);
        assert_eq!(boot.first_broken_seq, Some(2));
        log.append_action(&principal(), 0, read_action("act_2", "git.status"))
            .unwrap();
        // Never rewritten.
        assert_eq!(fs::read(&seg1).unwrap(), before);
        assert!(d.path().join("audit").join(segment_name(2)).exists());
    }
}
