# Pilot: running discover → characterize → draft-IR by hand on three real targets on this Mac

Research date: 2026-11-05

Evidence legend:
- **OBSERVED**: reproduced on this machine (macOS 27.0.1 build 26A434) or read in a primary file. Each item gives the command or the file path with line numbers.
- **SECONDARY**: third-party or search summaries. This pilot uses almost none.
- **INFERRED**: my interpretation, estimate or design proposal.
- **UNKNOWN**: not established.

REA is cited as `src/path.ts:line` against `/Users/nhanclie/rea` (HEAD `3484432fb`, 2026-11-05). Nothing in `/Users/nhanclie/rea` was modified.

**Why this exists.** Several existing reports make claims this pilot can test:
- `rea-adding-a-new-target-walkthrough.md` #10 says a target can be scaffolded "in minutes".
- `survey-capability-discovery-and-distillation.md` §7 says the deterministic stages do most of the work.
- `survey-agent-interface-generators.md` proposes a capability IR.
- `rea-tool-design-philosophy-and-verification.md` §5 R4 says auto-discovered effects default to all-true.

None of these was carried end to end on one target, and five reports propose five IR shapes. This pilot runs stages 1-2 of the discovery survey's pipeline (§7) by hand on three targets of different shapes, under `static`/`introspect` authority only. Writes ran only against throwaway fixtures. Every draft IR entry stops at `status: proposed` or, where a fixture allowed it, `fixture-verified`.

It does not repeat these reports; it cites them:
- the discovery survey: source catalogue §3, effect-signal table §4.3, pipeline §7;
- the generators survey: CLI-Anything, OpenAPI→MCP;
- the walkthrough: REA's slice cost;
- `rea-cli-surface-and-agent-packaging.md` §10.1: the `defineCapability` contract;
- the tool-design report: rubric §5, ladder §6;
- `survey-os-automation-apis.md`: Apple Events and App Intents as control APIs.

---

## Summary

1. **Harvest is cheap. Stage 1 always produced usable raw output.** OBSERVED:
   - **gh 2.102.0:** 428 process invocations, about 520 KB raw, 197 leaf commands with 1,014 flags. The `__complete` crawl took 16.0 s for 230 calls.
   - **Music 1.7:** two static files, 102 KB. They give 31 commands, 171 properties and 8 App Intents.
   - **Node 25.6.0 inspector:** one HTTP GET, 98 KB. It gives 107 commands and 43 events.

   No harvest step needed network access, credentials or user data.
2. **None of the three targets declares effects.** OBSERVED, as declared effect fields per surface:
   - gh/Cobra: 0 of 197.
   - CDP: 0 of 107. The command keys are only `name, description, parameters, returns, experimental, deprecated, redirect`.
   - Music sdef: 0 explicit effect fields. It does have **access-groups**, which work as a usable proxy: on 18 of 31 commands and on 20 of 26 classes.
   - Music App Intents: 2 of 8 carry an effect-bearing `systemProtocols` value. Notes App Intents: 29 of 51.

   Every other effect assignment in this pilot came from tables I wrote (a verb lexicon, Standard Suite semantics, an access-group map) or from my reading of prose. Discovery §4.3 frames effect classification as a separate, conservative step, and this result supports that. It also shows the step is mostly **reusable per-family tables plus per-target review**. It is not derived from the source.
3. **"Effect-UNKNOWN" needs two counts, and the reports so far merge them.**
   - Counted on declared signals only, almost everything is unknown: gh 197/197, CDP 107/107, Music App Intents 6/8.
   - After the floor tables, the residue is small: gh 11 unknown, CDP 1, Music sdef 1 command.

     But a manual review found the floor tables wrong in visible ways. In gh, `search code` and `run watch` were classed as writes, and 7 destructive flag branches sat inside "write" commands. In CDP, `Profiler.takePreciseCoverage` "resets execution counters" (prose only), and `Debugger.setScriptSource` edits live code but carries a session-looking verb.

   So "unknown" must stay a first-class value per effect bit, and the tables only *propose* values.
4. **Read-back oracles exist on all three surfaces, but at capability level, not field level.**
   - **gh:** 118 of 120 writes have a same-noun read command, and 88 of those reads have typed-ish `--json` output. Only **62 of 301** write flags name-match a JSON field (`--title`↔`title` works; `--enable-issues`↔`hasIssuesEnabled` does not). All flags matched for only 6 of 88 writes.
   - **sdef:** every `rw` property is its own oracle (implicit `get` after `set`). Playback commands pair with the read-only `player state` property.
   - **CDP:** `Runtime.callFunctionOn` was confirmed on a fixture by reading back with `Runtime.getProperties`. Breakpoints have **no** read-back command (`Debugger.getBreakpoints` → `-32601 wasn't found`). Removing a bogus breakpoint id returned success `{}`.

   Writes without a field-level oracle are the norm. REA's confirmed-write admission (`docs/agent-environments.md:175-186`) will withhold most of them unless the oracle mapping is hand- or LLM-authored.
5. **Schema fidelity varies by surface.**
   - **CDP:** fully typed. All 169 parameters carry `type`/`$ref`, and 43 commands have typed returns.
   - **sdef:** typed but opaque. 11 of 31 command direct parameters are `specifier`, which is an object-query language, not a JSON type. Ranges such as `rating` "(0 to 100)" are prose (`com.apple.Music.sdef:461`).
   - **App Intents:** typed but coded. Primitives are numeric `typeIdentifier` 0/1/12, and titles are localization keys such as `PlayMusicItemSiriIntent_Title`.
   - **gh/Cobra:** names typed, values semi-typed:
     - bool versus value is certain (282 bool flags);
     - 252 of 1,014 flags show a doc varname (`expression`, `fields`, `file`, `number`, `date`) where the type should be;
     - 59 enums and 76 defaults exist only in prose;
     - 143 of 197 leaves have positional grammars written as prose (`[<number> | <url> | <branch>]`).

   Output types: CDP is typed. gh `--json` gives field names only, with no types. sdef gives a result type on 8 of 31 commands. App Intents `outputType` is set on 1 of 8 (Music) and 22 of 51 (Notes).
6. **Probing surfaced facts no description carries.** OBSERVED on fixtures:
   - CDP silently ignores unknown parameters (`Runtime.evaluate {bogus:1}` succeeded). Closed schemas must therefore be enforced by the adapter, not delegated to the target.
   - CDP rejects wrong types (`-32602`), so parser-conformance probing works.
   - `Profiler.start` needs an undeclared `Profiler.enable` precondition (`-32000 Profiler is not enabled`).
   - gh uses exit code 1 for both usage errors and runtime errors, and 4 for "auth needed".
   - `Schema.getDomains` at runtime lists 5 domains, but `/json/protocol` lists 14 for the same process. The target's own self-descriptions disagree.
   - Writing to a fresh gh profile created a whole config file (`config.yml` with default alias `co`), so `alias delete` is not the inverse of `alias set` at the file level.
7. **The "minutes" claim holds for `scaffolded`, not for `audited`.** INFERRED from §8.
   - Harvest plus deterministic parse takes seconds once a per-family parser exists. The pilot's parsers were 20-40 lines each.
   - Getting to honest unknowns, closed input schemas and effect floors takes minutes.
   - Getting to "effects audited and write oracles paired" took roughly 300 atomic judgments for gh, about 60 for Music and about 60 for the CDP host. About half of gh's are per-capability field mappings. That is hours per target, not minutes.
8. **The merged IR has 38 candidate fields across the five reports. The pilot used 21 of them** (§7). The rest fall into three groups:
   - 7 belong to projections (CLI hints, skill hints, summary, title);
   - 5 duplicate other fields (`authority` vs effects, `idempotent` twice, annotations);
   - 5 had no source anywhere (`reversible`, `undo_binding`, `retainedSummary`, `coverage`, `licence` at discovery time).

   Four fields that appear only weakly or not at all in the existing shapes were essential: **per-bit tri-state effects with provenance**, **effect branches** (flag- or method-conditioned), **oracle with kind** (`read-back | event | negative | cross-surface | none`), and **preconditions as data**. The recommended minimal IR is at the end.

---

## 1. Method

**Machine and targets.** The machine is the one in the header. "Authorized" means software the owner installed and runs locally. No account or remote state was touched.

| Shape | Target | Version / identity (OBSERVED) | Why chosen |
|---|---|---|---|
| (a) Cobra CLI | `gh` | 2.102.0; `sha256(/opt/homebrew/Cellar/gh/2.102.0/bin/gh)=81904d0a…32f1` | Large Cobra tree; remote-state CLI; has local-only writes usable on a fixture |
| (a′) Swift ArgumentParser CLI (cross-check) | `/usr/bin/shortcuts` | macOS 27.0.1 | Tier-0 JSON dump to compare with Cobra |
| (b) scriptable Mac app | Music.app (`com.apple.Music`) | `CFBundleShortVersionString=1.7`; `sha256(com.apple.Music.sdef)=ebc83706…e664`; `sha256(extract.actionsdata)=62aa7407…f521`; App Intents `toolsVersion 27A200c` | Both an sdef and App Intents on one app, so two surfaces can be compared |
| (b′) comparison | Notes.app | same OS | App Intents density, sdef `xi:include` |
| (c) CDP service I start | `node --inspect=127.0.0.1:9339 fixture.js` | node v25.6.0, `sha256(bin/node)=8b6a6d43…4202`; `sha256(/json/protocol)=efd1be75…5940` | Real CDP self-description from an installed binary; the fixture process makes write probes safe |

**Scrubbed environment (OBSERVED working).** Every probe ran through a wrapper:
- `env -i HOME=<scratch>/home PATH=/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin GH_NO_UPDATE_NOTIFIER=1 GH_PROMPT_DISABLED=1 NO_COLOR=1 DO_NOT_TRACK=1`;
- `sandbox-exec -p '(version 1)(allow default)(deny network-outbound)(allow network-outbound (remote ip "localhost:*"))(allow network-outbound (remote unix-socket))'`.

`curl https://example.com` under the wrapper failed with exit 7 (blocked). The gh probes left exactly one file in the scratch HOME (`.config/gh/config.yml`), and only after the fixture write. Nothing was written outside the scratch HOME (`find` on the scratch HOME after all runs).

**Failed attempt, recorded because it is a lane-design fact (OBSERVED).** I first tried VS Code 1.142.0 (Electron) as the CDP target, with `--remote-debugging-port` and a throwaway `--user-data-dir`. It failed twice:
1. Under `sandbox-exec`, Chromium's own sandbox reported `sandbox initialization failed: Operation not permitted` (nested Seatbelt).
2. Electron refused the profile: `IPC handle "<scratch>/cdp/ud/1.14-main.sock" is longer than 103 chars`, then `listen EINVAL`.

DevTools had started listening (`DevTools listening on ws://127.0.0.1:9333/…`) before the crash. I switched to the Node inspector, which serves the same `/json/protocol` shape.

**Counting rules.**
- A *candidate capability* is one addressable operation:
  - a Cobra leaf command;
  - an sdef command;
  - an implicit sdef `get`/`set` on one class property;
  - an App Intent;
  - a CDP command.
- An effect is *declared* only if the target's own description carries a field with effect meaning.
- A *floor* is what a reusable per-family table assigns.
- *Unknown* means neither applies.

## 2. Target (a): `gh` (Cobra) plus the `shortcuts` cross-check

### 2.1 Harvest (OBSERVED)

| Probe | Authority | Calls | Raw bytes | Yield |
|---|---|---|---|---|
| `gh reference` (built-in markdown of the whole tree) | introspect | 1 | 94,905 | 229 headings: 32 groups and 197 leaves; flag tables with type tokens |
| `gh __complete <group> ""` and `gh __complete <leaf> --` | introspect | 230 (16.0 s) | 120,724 as JSON | Subcommand sets plus flag descriptions. They matched `gh reference` exactly, except that completion adds `help` at the root |
| `gh <leaf> --help` | introspect | 197 | 303,348 | `USAGE`, `ALIASES`, `FLAGS`, `INHERITED FLAGS`, **`JSON FIELDS`** (39 leaves), `EXAMPLES` (118 leaves) |
| Fresh `config.yml` written by gh after the first local write | (by-product) | — | 1,784 | Self-documented settings with enums ("Supported values: ssh, https"). This is a config-schema source (discovery §3.5) |
| `shortcuts --experimental-dump-help` | introspect | 1 | 9,869 | 4 leaves (`run`, `list`, `view`, `sign`). Each arg has `kind`, `names`, `isOptional`, `isRepeating`; `sign --mode` has `defaultValue` |

Dynamic completion stayed inert offline. `gh __complete pr view ""` returned directive `:0` with no candidates and no error. `gh __complete pr list --state ""` returned the static enum `open closed merged all`. Unlike the discovery survey's kubectl case, gh did not try the network, but it had no token to try with (OBSERVED; behaviour with a token is UNKNOWN).

Two independent self-descriptions agree on the tree (reference vs `__complete`). This cross-check is deterministic and free, so a harvester should always run it (INFERRED).

### 2.2 Schema fidelity (OBSERVED counts from parsing `gh reference`)

| Item | Count | Fidelity |
|---|---|---|
| Leaf flags | 1,014 | Name, short and description: declared |
| Bool flags (no type token) | 282 | Typed, certain |
| `string` 407, `int` 31, `strings` 32, `stringArray` 5, `duration` 3 | 478 | Typed. `int` was confirmed by a parser probe: `gh pr list --limit notanint` → `strconv.ParseInt … invalid syntax`, exit 1 |
| Varname tokens replacing the type (`expression` 60, `fields` 39, `file` 23, `number` 20, `name` 18, `date` 12, `login` 11 …) | 252 | Semi-typed. Cobra's `UnquoteUsage` lets a back-quoted word in the description replace the type name, so the real type (almost always string) is hidden (INFERRED from Cobra behaviour) |
| Enum in prose `{a\|b\|c}` | 59 flags | Prose. Also available through `__complete <leaf> --flag ""` for some flags |
| Default in prose `(default …)` | 76 flags | Prose |
| Leaves with positionals in USAGE | 143 of 197 | Prose grammar, e.g. `gh agent-task view [<session-id> \| <pr-number> \| <pr-url> \| <pr-branch>]` |
| Output schema | 39 leaves list `JSON FIELDS` (e.g. `pr list` has 46 fields) | Names only, no types |

Swift ArgumentParser gives cleaner structure (kind, optional, repeating) but no value types either. It also gives no effect field and no output schema (OBSERVED `shortcuts` dump).

### 2.3 Effect signals (OBSERVED presence; mapping INFERRED)

**Declared effect fields: none.** Weak native signals found in flags and prose:

| Signal | Leaves | What it means |
|---|---|---|
| `--yes` / `--confirm` | 15 | gh's non-interactive confirmation for deletes. On `issue comment` it guards the `--delete-last` branch (help: "Skip the delete confirmation prompt when --delete-last is provided") |
| `--force` | 11 | On `repo sync`: "Hard reset the branch of the destination repository". On `label create`/`clone`: overwrite |
| `--dry-run` | 4 | `extension upgrade`, `pr create`, `skill publish`, `skill update` |
| `--web` | 38 | Opens a browser (`launchesProcess`/`changesUiState`). Present on 36 read commands, so read tools gain a side-effecting branch |
| `--json` | 40 | Structured output, a read cue |
| Destructive keywords in description | 21 | delete/remove/… |
| `gh api -X/--method` (default GET) | 1 | Effect depends on an argument. This is mixed authority (tool-design R3) |

**Floor via a verb lexicon I wrote for this pilot** (about 110 verbs; a human artifact, not discovered). Result: read 66, write 99, destructive 21, **unknown 11**. The unknowns are `api`, `co` (the default alias), `completion`, `config clear-cache`, `copilot`, `preview prompter`, `release verify-asset`, `repo read-dir`, `repo read-file`, `repo set-default` and `skill publish`.

**Manual review of the floor found these errors:**
- **Wrong level (5):** `search code` and `run watch` are reads but were classed as writes. `browse`, `extension browse` and `codespace code` are UI or process launches, not target writes.
- **Remote versus local confusion (about 8):** `repo clone`, `gist clone`, `run download`, `release download`, `attestation download`, `codespace cp` and `pr checkout` write the local filesystem, not GitHub. One level cannot say which.
- **Flag-raised destructive branches inside "write" (7):**
  - `issue comment`, `pr comment` and `discussion comment` with `--delete-last`;
  - `repo sync --force`;
  - `label create --force` and `label clone --force`;
  - `pr checkout --force`.
- **Network:** no gh command declares `accessesNetwork`. I set it true by default with about 15 local-only exceptions (`alias *`, `config *`, `completion`, `help`, …). This is one target-wide judgment plus a list of exceptions.

### 2.4 Oracles (OBSERVED by pairing; INFERRED sufficiency)

- **Capability-level pairing:** for each of the 120 write or destructive leaves, I looked for a read leaf under the same noun:
  - 88 have a read with `--json`;
  - 30 have a prose-only read (`auth status`, `alias list`, …);
  - 2 have none (`codespace ports forward`, `codespace ports visibility`).
- **Field-level pairing:** only 62 of 301 non-generic write flags name-match a JSON field of the sibling read. All flags matched on just 6 writes. Examples:
  - `issue edit`: 4 of 6 matched (`attach` and `type` did not);
  - `repo edit`: 3 of 12 matched (`--enable-issues` ↔ `hasIssuesEnabled` needs a semantic map).
- **Negative oracles** are needed for deletes (`repo view` must fail after `repo delete`). No description says which exit code or message means "absent". Only `4` = auth was OBSERVED; the not-found behaviour is UNKNOWN offline.

### 2.5 Fixture write probe (OBSERVED, scratch HOME only)

| Step | Result |
|---|---|
| Snapshot the scratch HOME file tree (sha256) | empty |
| `gh alias set pv 'pr view'` | exit 0. Diff: **+ `.config/gh/config.yml`**, a whole file with defaults and alias `co: pr checkout` |
| `gh alias list` (read-back) | `co: pr checkout` / `pv: pr view` (prose `name: expansion`). Snapshot unchanged, so `observed ⊆ declared` holds for the read |
| `gh config set editor vim` → `gh config get editor` | `vim` (typed, scalar read-back) |
| `gh alias delete pv` → `gh alias list` | `co: pr checkout`. The file still exists, with a new hash. Delete does not invert set at the file level |
| `gh pr list --bogus` / `--limit notanint` / no auth | exit 1 / exit 1 / exit 4 (`To get started with GitHub CLI, please run: gh auth login`) |

### 2.6 Draft IR entries (gh)

These use the minimal IR recommended at the end. Comments give the provenance class.

```yaml
target: {id: gh, surface: cli, identity: {exe_sha256: "81904d0a…32f1", version: "2.102.0"},
         preconditions: [{id: gh.auth, kind: credential, probe: "gh auth status", observed_exit_when_missing: 4}],
         discovery: {authority: introspect, sources: [gh-reference, cobra-complete, help]}}
capabilities:
- id: gh.pr.list
  address: {argv: [gh, pr, list]}
  description: {text: "List pull requests in a GitHub repository…", trust: untrusted}
  input_schema:   # closed
    type: object
    additionalProperties: false
    properties:
      state: {enum: [open, closed, merged, all], default: open}   # enum: declared (__complete); default: prose
      limit: {type: integer, default: 30}                          # declared token "int" + parser probe (observed)
      label: {type: array, items: {type: string}}                  # declared "strings"
      search: {type: string}                                       # varname "query": type inferred
      json: {type: array, items: {enum: [additions, assignees, author, "… 46 fields"]}}
  excluded_inputs: [{name: web, reason: "launches browser: crosses effect class (R3)"}]
  output_schema: {kind: field-names, fields: 46, types: unknown}
  effects: {mutatesTarget: false, accessesNetwork: true, launchesProcess: false, writesFilesystem: false,
            mayDiscardData: false, idempotent: true}
  effect_provenance: {mutatesTarget: floor(verb:list), accessesNetwork: target-default}
  preconditions: [gh.auth, {kind: context, text: "a repo via cwd or --repo"}]
  oracle: {kind: self}          # a read is its own observation
  status: proposed
- id: gh.alias.set
  address: {argv: [gh, alias, set, "{name}", "{expansion}"]}
  input_schema: {type: object, additionalProperties: false, required: [name, expansion],
                 properties: {name: {type: string}, expansion: {type: string}, shell: {type: boolean}, clobber: {type: boolean}}}
  effects: {mutatesTarget: false, writesFilesystem: true, accessesNetwork: false, mayDiscardData: unknown, idempotent: unknown}
  effect_provenance: {writesFilesystem: observed(fixture diff +.config/gh/config.yml)}
  branches: [{when: {clobber: true}, effects: {mayDiscardData: true}, provenance: inferred(flag name)}]
  oracle: {kind: read-back, capability: gh.alias.list, match: "line '<name>: <expansion>'", output: prose, status: fixture-verified}
  status: fixture-verified
- id: gh.label.create
  address: {argv: [gh, label, create, "{name}"]}
  effects: {mutatesTarget: true, accessesNetwork: true, mayDiscardData: false, idempotent: false}
  branches: [{when: {force: true}, effects: {mayDiscardData: true}, provenance: "desc: update if exists"}]
  oracle: {kind: read-back, capability: gh.label.list, fields: {color: color, description: description, name: name}, status: proposed}
  status: proposed
- id: gh.repo.delete
  address: {argv: [gh, repo, delete, "{repo}", --yes]}
  effects: {mutatesTarget: true, mayDiscardData: true, accessesNetwork: true, idempotent: unknown}
  effect_provenance: {mayDiscardData: "floor(verb:delete) + flag(--yes) + desc keyword"}
  oracle: {kind: negative, capability: gh.repo.view, expect: not-found, signal: unknown}   # not-found exit/message UNKNOWN
  dry_run: none
  status: proposed
- id: gh.api
  status: withheld
  withheld_reason: mixed-authority          # -X/--method selects read vs write vs destructive
  next_probes: [{operation: "split into gh.api.get (method fixed GET) + per-route writes", confidence: inferred}]
```

### 2.7 Human-judgment steps (gh), estimated (INFERRED)

| Judgment | Count | Reuse |
|---|---|---|
| Pick harvest sources and write the Cobra parsers (reference, complete, help) | 3 parsers | per family (Cobra) |
| Verb lexicon | ~110 entries | partly reusable across Cobra CLIs |
| Review lexicon output: 11 unknown, 7 flag branches, 5 wrong level, ~8 local-vs-remote | ~31 | per target |
| Network default and local-only exceptions | 1 + ~15 | per target |
| Varname-token → type map | ~30 distinct tokens covering 252 flags | per family |
| Positional grammars → schema | 143 | per target (LLM-assistable, parser-probe checkable) |
| Field-level oracle maps | 88 writes, 239 unmatched flags | per capability (dominant cost) |
| Negative-oracle signal for deletes | ~21 | per target, needs one probe each against a fixture repo (not possible offline) |
| Auth precondition and secret handling | 1 | per target |
| Core-set selection (~30 of 197) | 1 | per target |

Total: roughly 300 atomic judgments, of which about 150 are per-capability.

## 3. Target (b): Music.app (sdef plus App Intents), with Notes for comparison

### 3.1 Harvest (OBSERVED, static only)

| Source | Path | Bytes | Yield |
|---|---|---|---|
| `Info.plist` | `OSAScriptingDefinition=com.apple.Music.sdef`, `NSAppleScriptEnabled=true` | — | Says where the dictionary is |
| sdef | `/System/Applications/Music.app/Contents/Resources/com.apple.Music.sdef` | 45,766 | 3 suites; **31 commands** (Standard 12, Music 18, Internet 1); 26 classes (24 inherit); **171 class properties** (62 `r`, 109 `rw`); 13 enumerations; no `xi:include` |
| App Intents | `…/Metadata.appintents/extract.actionsdata` | 56,544 | **8 actions**, 12 entities, 11 queries, 8 enums |
| Notes sdef | `Notes.sdef` | 8,575 | 2 own commands plus `xi:include` of `CocoaStandard.sdef` (12,308 B, 11 commands) at `Notes.sdef:6` |
| Notes App Intents | `extract.actionsdata` | 184,933 | 51 actions |

Naive candidate count for Music: 31 commands, 171 implicit gets and 109 implicit sets, so **311**, plus 8 App Intents. Implicit `get`/`set` are not commands in the sdef. They come from AppleScript's object model over `property access` (OBSERVED: no `get`/`set` command element in the file).

### 3.2 Schema fidelity

- **sdef:** every parameter has a type. But 11 of 31 direct parameters are `specifier` and 4 are `location specifier` (OBSERVED type counts). A specifier is a query (`track 3 of playlist "X"`, `every track whose …`), not a value. The JSON Schema projection needs a design decision (UNKNOWN best form). Value ranges are prose: `rating` "(0 to 100)" at `com.apple.Music.sdef:461`, `sound volume` "(0 = minimum, 100 = maximum)" at `:265`. Enums are typed (e.g. `ePlS` stopped/playing/paused/fast forwarding/rewinding, `:518-524`). Result types are given on 8 of 31 commands.
- **App Intents:** `valueType` kinds:
  - Music: `alternative` 6, `entity` 2, `array` 2, `linkEnumeration` 2, `searchCriteria` 1;
  - Notes: `entity` 28, `primitive` 26, `linkEnumeration` 19, `array` 17.

  Primitives are numeric codes (Notes `AppendToNoteLinkAction`: `text` is typeIdentifier 12, `section` is 0, booleans are 1). No mapping table ships in the file (UNKNOWN). Music titles and descriptions are localization keys (`UpdateMusicItemAffinitySiriIntent_Title`) that must be resolved against `SiriAppIntents.strings`. Notes keys are already English (`"Delete Notes"`). `outputType` is set on 1 of 8 (Music) and 22 of 51 (Notes).

### 3.3 Effect signals (OBSERVED)

| Signal | Coverage | Mapping (INFERRED) |
|---|---|---|
| Command `access-group` | 18 of 31 commands. Groups: `com.apple.Music.library.read-write` rw (add, convert, download, refresh), `library.read` r (export, search), `playback` (10 transport commands), `user-interface` (reveal, select). Standard Suite carries only suite-level `<access-group identifier="*"/>` (`:5`) | library.read-write → `mutatesTarget`; library.read → read; playback → `changesUiState`/device-like; user-interface → `changesUiState`. These are TCC Apple-Event scoping groups; Apple does not document them as effect classes |
| Class/element/property `access-group` | 20 of 26 classes, 13 of 32 elements, 19 of 182 property nodes. Example: the `AirPlay device` element is `playback rw` (`:167-168`) | Refines which group a `get`/`set` belongs to |
| Property `access` | 62 `r`, 109 `rw` | get → read; set → write. Whether a set can discard data is unknown per property |
| Parameter `requires-access="r"` | 8 commands (print, count, duplicate, exists, move, play, reveal, select) | The direct object is read. It says nothing about the command's own effect |
| Standard Suite verbs | 12 | A fixed cross-app table: `delete`/`close`/`quit`/`move` → may discard; `make`/`duplicate`/`save` → write; `count`/`exists` → read; `print`/`open`/`run` → process or UI |
| App Intents `systemProtocols` | Music 2 of 8: `AudioStarting` (Play), `ShowInAppStringSearchResults` (Search). Notes 29 of 51: `OpenEntity` 9, `CreateEntity` 7, `DeleteEntity` 6, `ChangeBinarySetting` 2, `CloseEntity` 2, `PutEntityInContainer` 1, `FavoriteEntity` 1, … | As in discovery §4.3 |
| App Intents `assistantDefinedSchemas` | Music 8 of 8 (e.g. `{domain: audio, name: AddAudioToLibraryIntent}`, `UpdateAudioAffinityIntent`); Notes 3 | A cross-app *semantic verb* namespace. It is a stronger prior than the bare `AssistantIntent` protocol, but it is still a name, not an effect (INFERRED) |
| `openAppWhenRun`, `isDiscoverable`, `authenticationPolicy` | Music: 2 open the app; **0 of 8 discoverable**; all auth 0. Notes: 47 of 51 discoverable | Preconditions and reachability, not effects |

**Unknown after floors:**
- sdef commands: 1 (`open location`, which opens a store or stream URL: network or UI).
- App Intents by protocol: Music 6 of 8 and Notes 19 + 3 of 51. The Notes cases include `AppendToNote`, `RenameFolder`, `RemoveTagsFromNotes` and `AddOrRemoveNoteLock`, all obvious writes with no protocol.
- Property sets: `mayDiscardData` stays unknown for all 109 until a read-before-write policy covers it.

### 3.4 Oracles

- **Every `rw` property is self-oracled.** `set rating of t to 60` is checked by `get rating of t`. `delete` is checked by `exists`. `make`/`add` return a specifier or `track`, read back by `persistent ID` (`:349`, `access="r"`, "does not change over time").
- **Playback commands** (10) pair with `player state` (`:250`, `r`, typed enum) and `current track` (`:233`).
- **App Intents have no read side in the catalog.** Music's write intents (add to library or playlist, update affinity) need a **cross-surface oracle**: verify through the sdef (`favorited` `:450`, `disliked` `:436`). INFERRED. This is the differential pattern from discovery §6, used here as the only option.
- **Runtime verification was not possible in this pilot.** Any Apple Event to Music, including a pure `get player state`, triggers a TCC Automation prompt for the calling process and acts on the user's real library. With the owner away from the keyboard and a no-user-data rule, every Music oracle stays `status: proposed` (UNKNOWN whether they hold).

### 3.5 Draft IR entries (Music)

```yaml
target: {id: music, surface: apple-event, identity: {bundle_id: com.apple.Music, version: "1.7", sdef_sha256: "ebc83706…e664",
         appintents_sha256: "62aa7407…f521"},
         preconditions: [{id: tcc.automation.music, kind: os-consent, human: true}, {id: music.running, kind: process, launches_if_absent: true}]}
capabilities:
- id: music.player.playpause
  address: {aevt: hookPlPs, sdef: "playpause"}
  input_schema: {type: object, additionalProperties: false, properties: {}}
  effects: {changesUiState: true, mutatesTarget: false, mayDiscardData: false, idempotent: false}
  effect_provenance: {changesUiState: "declared access-group com.apple.Music.playback"}
  oracle: {kind: read-back, capability: music.application.player_state.get, expect: "toggles playing<->paused", status: proposed}
  status: proposed
- id: music.track.rating.set
  address: {aevt: "setd", property: pRte, class: cTrk}
  input_schema: {type: object, additionalProperties: false, required: [track, value],
                 properties: {track: {$ref: "#/specifier/track"}, value: {type: integer, minimum: 0, maximum: 100}}}  # range from prose :461
  effects: {mutatesTarget: true, mayDiscardData: unknown, idempotent: true}
  effect_provenance: {mutatesTarget: "declared property access rw (:461)", mayDiscardData: "unknown: prior value not kept"}
  oracle: {kind: read-back, capability: music.track.rating.get, status: proposed}
  undo: {kind: read-before-write, inferred: true}
  status: proposed
- id: music.item.delete
  address: {aevt: coredelo}
  input_schema: {type: object, additionalProperties: false, required: [item], properties: {item: {$ref: "#/specifier/any"}}}
  effects: {mutatesTarget: true, mayDiscardData: true, idempotent: unknown}
  effect_provenance: {mayDiscardData: "floor(standard-suite:delete)"}
  oracle: {kind: negative, capability: music.object.exists, expect: false, status: proposed}
  status: proposed
- id: music.playlist.search
  address: {aevt: hookSrch}
  input_schema: {type: object, additionalProperties: false, required: [playlist, for],
                 properties: {playlist: {$ref: "#/specifier/playlist"}, for: {type: string}, only: {$ref: "#/enum/eSrA"}}}
  output_schema: {type: array, items: {$ref: "#/specifier/track"}}
  effects: {mutatesTarget: false}
  effect_provenance: {mutatesTarget: "declared access-group library.read r"}
  oracle: {kind: self}
  status: proposed
- id: music.intent.update_affinity
  address: {app_intent: UpdateMusicItemAffinitySiriIntent, schema: "audio.UpdateAudioAffinityIntent"}
  effects: {mutatesTarget: unknown}
  effect_provenance: {mutatesTarget: "no effect-bearing systemProtocol; schema name suggests write (inferred)"}
  oracle: {kind: cross-surface, capability: music.track.favorited.get, status: proposed}
  status: withheld
  withheld_reason: not-invokable   # isDiscoverable=false; no third-party invocation path (survey-os-automation-apis.md §3.1)
```

### 3.6 Human-judgment steps (Music), estimated (INFERRED)

| Judgment | Count | Reuse |
|---|---|---|
| sdef parser, `xi:include` + xpointer resolution, class inheritance flattening | 1 | per family |
| Standard Suite effect table | 12 verbs | cross-app, once |
| Access-group → effect-bit map | 4 groups | per app (naming pattern `.read`/`.read-write`/`.playback`/`.user-interface` may generalize: UNKNOWN) |
| Specifier → JSON Schema design | 1, hard | per family |
| Prose ranges and units on properties | ~20 numeric props of 171 | per app |
| Playback oracle choice | 10 | per app |
| Read-before-write policy for sets | 1 | global |
| App Intents: string-table resolution, typeIdentifier table, discoverability exclusion | 3 | per family |
| TCC Automation grant | 1, must be a human at the keyboard | per (caller, app) pair |
| Core-set selection | 1 | per app |

Total: about 55-60 judgments. The sdef's own structure (r/rw, access-groups) does most of the work.

## 4. Target (c): Node 25.6.0 inspector over CDP (fixture process)

### 4.1 Harvest (OBSERVED)

| Probe | Result |
|---|---|
| `GET /json/version` | `{"Browser":"node.js/v25.6.0","Protocol-Version":"1.1"}` |
| `GET /json/list` | 1 target, `type: node`, `webSocketDebuggerUrl` on 127.0.0.1 |
| `GET /json/protocol` | 97,616 B. **14 domains**, 107 commands (51 experimental, 2 deprecated), 43 events, 169 parameters (all typed, 94 optional, 144 described), 27 commands without a description, 43 with typed returns |
| `Schema.getDomains` over the socket | **5 domains** (Runtime, Debugger, Profiler, HeapProfiler, Schema) |

The catalogue mismatch (14 versus 5) means "which self-description is authoritative" is a per-surface decision. Identity binding should hash the one actually used (INFERRED).

### 4.2 Effect signals and fidelity

- **Declared:** none on 107 of 107 commands. The command keys are `name`, `description`, `parameters`, `returns`, `experimental`, `deprecated`, `redirect`.
- **Verb floor (my table):** session-write 72, read 23, exec-foreign 7 (`evaluate`, `callFunctionOn`, `compileScript`, `runScript`, `evaluateOnCallFrame`, `restartFrame`, …), heavy or destructive 4 (`terminateExecution`, `collectGarbage`, `takeHeapSnapshot`, `takePreciseCoverage`), unknown 1 (`NodeWorker.sendMessageToWorker`).
- **Errors found by reading all `set*`/`get*`/`take*` descriptions:**
  - `Debugger.setScriptSource` "Edits JavaScript source live", and `setVariableValue`/`setReturnValue` change program state. These are target mutations under a session verb.
  - `Profiler.takePreciseCoverage` "…and resets execution counters": a read verb that mutates, stated only in prose.
  - `DOMStorage.setDOMStorageItem` is a persistent write.
  - About 25 `set*`/exec commands need a session-versus-target judgment.
- **Schema fidelity:** fully typed inputs and outputs. **Enforcement is not closed** (§4.3).

### 4.3 Fixture probes (OBSERVED, `inspectee.js` owned process with `globalThis.state={counter:1}`)

| Call | Response | Lesson |
|---|---|---|
| `Runtime.getProperties(state)` | `counter: [1]` | Typed read |
| `Runtime.callFunctionOn(state, "function(v){this.counter=v}", [42])` → `getProperties` | `counter: [42]` | **Write confirmed by an independent read-back.** Effect is `executesForeignCode` (arbitrary function) |
| `Runtime.evaluate {expression:"1+1", bogus:1}` | `{"value":2}` | Unknown parameters are **silently ignored**. The adapter must close the schema |
| `Runtime.evaluate {expression:5}` | `-32602 Invalid parameters … string value expected` | Types are enforced, so parser-conformance probes work |
| `Runtime.nope` | `-32601 'Runtime.nope' wasn't found` | Typed "unknown capability" error |
| `Debugger.setBreakpointByUrl` | `{breakpointId:"2:2:0:inspectee\\.js$", locations:[…]}` | Write returns an id… |
| `Debugger.getBreakpoints` | `-32601 wasn't found` | …but there is **no read-back command**. The only oracles are the returned id and the `Debugger.breakpointResolved` event |
| `Debugger.removeBreakpoint {breakpointId:"nope"}` | `{}` (success) | **Success does not prove effect**. Confirmed-write logic must not trust an empty success |
| `Profiler.start` (without `Profiler.enable`) | `-32000 Profiler is not enabled` | Undeclared precondition |

### 4.4 Draft IR entries (CDP)

```yaml
target: {id: node-inspector, surface: cdp, endpoint: {ws: "127.0.0.1:9339", loopback_only: true},
         identity: {exe_sha256: "8b6a6d43…4202", protocol_sha256: "efd1be75…5940", authoritative_catalog: "/json/protocol"},
         lifetime: session}
capabilities:
- id: node.runtime.get_properties
  address: {method: Runtime.getProperties}
  input_schema: {type: object, additionalProperties: false, required: [objectId],
                 properties: {objectId: {$ref: "#/Runtime.RemoteObjectId"}, ownProperties: {type: boolean}}}
  output_schema: {$ref: "#/Runtime.getProperties.returns"}
  effects: {mutatesTarget: false, mutatesSession: unknown}    # may allocate remote object handles
  preconditions: [{kind: session-handle, from: [node.runtime.evaluate]}]
  oracle: {kind: self}
  status: proposed
- id: node.runtime.call_function_on
  address: {method: Runtime.callFunctionOn}
  effects: {executesForeignCode: true, mutatesTarget: true, mayDiscardData: unknown, idempotent: false}
  effect_provenance: {executesForeignCode: "floor(verb:call) + desc", mutatesTarget: "observed fixture"}
  oracle: {kind: read-back, capability: node.runtime.get_properties, status: fixture-verified}
  status: fixture-verified
- id: node.debugger.set_breakpoint_by_url
  address: {method: Debugger.setBreakpointByUrl}
  effects: {mutatesSession: true, mutatesTarget: false, changesUiState: false, idempotent: false}
  preconditions: [{kind: call, capability: node.debugger.enable}]
  oracle: {kind: event, event: Debugger.breakpointResolved, or_return: breakpointId, status: proposed}
  note: "no list command; removeBreakpoint(bogus) returns {}: delete has no oracle"
  status: proposed
- id: node.profiler.take_precise_coverage
  address: {method: Profiler.takePreciseCoverage}
  effects: {mutatesSession: true, mutatesTarget: false}
  effect_provenance: {mutatesSession: "prose: 'resets execution counters' (verb floor said read)"}
  preconditions: [{kind: call, capability: node.profiler.enable}, {kind: call, capability: node.profiler.start_precise_coverage}]
  status: proposed
```

### 4.5 Human-judgment steps (CDP host), estimated (INFERRED)

The pass covers:
- the verb table (~35 verbs, reusable across CDP hosts);
- session-versus-target review (~25);
- reading 80 descriptions for prose-only effects (1 pass);
- `X.enable` preconditions (~9 domains);
- write oracles: ~15 have none, so each needs an event oracle or is withheld;
- the authoritative-catalog choice (1);
- core-set curation (1).

Total: about 60-70 judgments. These are mostly reusable for any CDP host (Chromium, Electron, Deno), because the protocol is shared. Electron launch hygiene (no nested sandbox, short profile path) is a per-family runtime concern.

---

## 5. Cross-target scorecard

| Metric | gh (Cobra) | Music (sdef + App Intents) | Node inspector (CDP) |
|---|---|---|---|
| Raw harvest | ~520 KB, 428 calls | 102 KB, 0 calls (files) | 98 KB, 3 GETs + 1 socket call |
| Candidate capabilities | 197 leaves (+4 `shortcuts`) | 31 commands + 171 gets + 109 sets + 8 intents = 319 | 107 commands (+43 events) |
| Declared effect fields | 0 | access-groups on 18/31 cmds; property r/rw on 171; protocol on 2/8 intents | 0 |
| Effect-UNKNOWN (declared only) | 197 | ~13 cmds + 6 intents | 107 |
| Effect-UNKNOWN after floor tables | 11 (+~20 needing review) | 1 cmd + 6 intents (+109 `mayDiscardData`) | 1 (+~25 needing review) |
| Writes with a read-back candidate | 118/120 (88 typed-ish) | all property sets; all playback; 0 intents natively | callFunctionOn yes; breakpoints no |
| Field-level oracle mapping | 62/301 flags auto-matched | trivial (same property) | n/a for exec; none for breakpoints |
| Input fidelity | names typed; 252 varname-typed; 59 prose enums; 143 prose positionals | typed, but `specifier`-heavy; prose ranges | fully typed; enforcement open |
| Output fidelity | field names only (39 leaves) | 8/31 result types | 43 typed returns |
| Runtime verification done | local writes on fixture HOME | none (TCC + user data) | fixture process |
| Human judgments (est.) | ~300 | ~55-60 | ~60-70 |

## 6. Which pipeline stages produced usable output

Stage names follow discovery §7.

| Stage / sub-step | Verdict | Evidence |
|---|---|---|
| 1 Discover: install inspection (static) | **Usable as-is** | Music/Notes files, `Info.plist` keys |
| 1 Discover: self-description calls (introspect) | **Usable as-is.** Two sources per target cross-checked for free | gh reference = `__complete` tree; CDP `/json/protocol` vs `Schema.getDomains` disagreed, which caught a catalog issue |
| 2 Characterize: normalize, resolve includes, identity binding | **Usable**, with a parser per family (20-40 lines each here) | sha256 of binary, sdef, protocol |
| 2 Characterize: schema projection from typed sources | **Usable for CDP; partial for sdef** (specifiers) **and Cobra** (varnames, prose enums, positionals) | §2.2, §3.2 |
| 2 Characterize: effect floors from native signals | **Usable only with hand-written per-family tables**, and the tables had errors | §2.3, §4.2 |
| 2 Characterize: oracle pairing | **Capability-level usable** (same noun). **Field-level mostly not** (21% name match) | §2.4 |
| 4 Verify: parser-conformance probe | **Usable and cheap** (gh exit 1, CDP -32602) | §2.5, §4.3 |
| 4 Verify: observed ⊆ declared via snapshot | **Usable on file-backed fixtures**. It found an undeclared side effect (whole config file created) | §2.5 |
| 4 Verify: confirmed-write | **Worked where a read exists**. Two hazards: empty success on a bogus delete; no read command for breakpoints | §4.3 |
| Any stage for a GUI-consent surface (Apple Events) | **Blocked** without a human TCC grant and a non-user library | §3.4 |

The discovery survey's §7 sizing ("steps 1, 2 (typed half), 4 and 5 are ordinary engineering") holds for discovery and typed projection. **It does not hold for effect floors or field-level oracles.** Those are judgment-heavy even on typed surfaces, because no surface here declares effects or oracle field maps (INFERRED from §2-4).

## 7. Merged candidate IR (from five reports) and pilot usage

Sources:
- **D**: discovery §2 `Capability` record;
- **G**: generators #2 `capabilities.yaml`;
- **W**: walkthrough #1 `target.yaml` `operations[]`;
- **C**: CLI §10.1 `defineCapability`;
- **T**: REA `ToolContract` (`src/contracts/toolContractTypes.ts:37-65`) plus rubric R1/R11/R15/R21.

"Used" means it was filled or was essential in the §2-4 drafts.

| # | Field | From | Pilot | Note |
|---|---|---|---|---|
| 1 | `target.id` / `display_name` | W | USED | |
| 2 | `target.identity` (exe/bundle/spec hash, version) | D, W `version_probe` | USED | All three. Hashing *both* bundle version and file digest caught nothing here but is cheap |
| 3 | `target.surface` / `shape` / `kind` / `binding` | D, W, G, T | USED | One concept under four names. Keep `surface` on the target and `address` on the capability |
| 4 | `target.endpoint` + locality | W | USED | CDP loopback |
| 5 | `target.settings[]` | W | UNUSED | Became a precondition (`gh.auth`) |
| 6 | `target.licence` | W | UNUSED | Packaging-time, not discovery-time |
| 7 | `target.host_support[]` | W | USED (weak) | sdef/App Intents are macOS-only |
| 8 | `id` | all | USED | `<app>.<noun>.<verb>` worked for all three |
| 9 | `title` | G, C, T | UNUSED | REA derives it from the name (`src/contracts/toolEffects.ts:824-828`) |
| 10 | `summary` | C | UNUSED | Projection |
| 11 | `description` | G, C, T | USED | Harvested, marked untrusted |
| 12 | `address` / upstream mapping | D, G `binding`, W | USED | argv template, `aevt` code, CDP method, intent id |
| 13 | `input_schema` (closed) | all | USED | Closure must be adapter-enforced (CDP ignores unknowns) |
| 14 | `output_schema` | D, G, C, T | USED | Needed a `field-names` / `unknown` form for gh |
| 15 | `effects` (REA 10 booleans) | G, W, C, T | USED, **needed tri-state** | `unknown` per bit |
| 16 | effect level (`read\|ui\|write\|destructive\|unknown`) | D | USED as derived | Not stored |
| 17 | per-signal effect provenance | D | USED, essential | Every bit needed a source |
| 18 | `annotations` | G, T | UNUSED as stored | Derive with `annotationsFromEffects` (`src/contracts/toolEffects.ts:765-786`) |
| 19 | `authority` (observe/launch/attach/mutate) | G, C | UNUSED (redundant) | Derivable from effects. Discovery authority is a *target-run* property |
| 20 | `idempotent` (separate) | G | UNUSED (duplicate of the effects bit) | |
| 21 | `reversible` / `undo_binding` | G | UNUSED | No source carried it. Only inferred read-before-write |
| 22 | `preconditions` | D (T `requiresActiveTarget`) | USED, essential | auth, TCC, running app, `X.enable`, session handle |
| 23 | `oracle` / `verification` refs | D, G | USED, essential, **needed `kind`** | read-back / self / negative / event / cross-surface / none |
| 24 | provenance / confidence (`declared\|observed\|inferred`) | D, G, W `confidence` | USED | Per field, not per entry |
| 25 | `returns_target_content` | W, C, T | USED (constant true) | Default true, as REA does (`src/contracts/toolEffects.ts:788-816`) |
| 26 | `lifetime` (one-shot\|session) | C | USED | CDP handles and breakpoints are session-scoped |
| 27 | `paging` | C | USED (weak) | gh `--limit` only |
| 28 | `errors` | C, T R15 | USED (observed by probe, not discovered) | gh 1/4, CDP -32601/-32602/-32000 |
| 29 | `cli` hints | C | UNUSED | Projection |
| 30 | `skill` hints / use_when | C | UNUSED | Projection |
| 31 | `examples` | C, T | AVAILABLE, UNUSED | 118 gh leaves ship EXAMPLES; useful as probe seeds later |
| 32 | `retainedSummary` | T | UNUSED | REA-specific |
| 33 | `coverage` | T R11 | UNUSED in IR | A result-envelope field, not a capability field |
| 34 | `family` | T R21, D §4.4 | USED (weak) | Cobra groups and sdef suites give it for free |
| 35 | `dry_run` | D §4.3/#8 | USED | gh: 4 commands |
| 36 | per-branch effects (flag/method-conditioned) | T R3 (as a lint) | USED, essential | 7 gh branches, `gh api` |
| 37 | `status` lifecycle | W, G #9 | USED | proposed / fixture-verified / withheld |
| 38 | `withheld_reason` + `next_probes` | D (REA `next_probes`, `src/domain/desktopApplication.ts:207`), REA withheld reasons (`docs/agent-environments.md:175-186`) | USED | not-invokable, mixed-authority |

Tally: 21 used, 7 projection-only, 5 redundant or derivable, 5 with no source in the pilot. #36 is the only field present in *none* of the five shapes as a data field. It exists only as a lint in R3.

**On tool-design R4 ("default every effect of an auto-discovered action to true").** In the pilot this would have marked all 197 gh leaves, all 319 Music candidates and all 107 CDP commands as `destructiveHint=true` and `openWorldHint=true` (by the formula at `src/contracts/toolEffects.ts:779-785`). For *policy* that is equivalent to "ask". But it erases the difference between "audited destructive" (e.g. `gh repo delete`) and "never looked at", and the audit would then be recorded as "narrowing". INFERRED recommendation:
- store `unknown` per bit;
- let policy and annotation derivation map `unknown → true`;
- keep the narrowing record (who or what lowered a bit, and on what evidence).

REA has no `unknown` because every one of its tools is hand-audited (`Missing effect audit`, `src/contracts/toolEffects.ts:822`). A discovery kit does not have that luxury.

## 8. Human judgment and the "minutes" claim

| | gh | Music | CDP host |
|---|---|---|---|
| Per-family (one-time) judgments | ~145 (parsers, ~110-verb lexicon, ~30 varname map) | ~20 (parser, Standard Suite, specifier design, intents tables) | ~40 (verb table, preconditions model) |
| Per-target judgments | ~50 | ~25 (+1 TCC human-at-keyboard) | ~25 |
| Per-capability judgments | ~250 (143 positionals, 88 oracle maps, ~21 negative oracles) | ~30 (ranges, playback oracles) | ~15 (oracles) |
| Wall-clock harvest | 16 s crawl + 197 help calls (secs) | <1 s | <1 s |

INFERRED estimate. With per-family parsers and tables already in the kit, reaching `scaffolded` takes minutes: closed input schemas, effect floors with explicit unknowns, capability-level oracle candidates, and `withheld` entries with reasons. Reaching `audited` (every bit decided, every write field-level oracled) is dominated by per-capability work. For a 200-command CLI that is hours of reviewing LLM proposals, not minutes. A small sdef app is closest to "minutes", because r/rw and access-groups carry most of the signal. The walkthrough's three-state status (`scaffolded | live-verified | hardened`) is the right way to present this.

---

## What to distill for agentic-anything

1. **Recommended minimal IR** (everything else is a projection or derivable):

   ```yaml
   target:
     id: string
     surface: cli | apple-event | app-intent | cdp | http | dbus | com | a11y | plugin-bridge
     identity: {version?: string, digests: {<artifact>: sha256}}          # binary, sdef, protocol, spec
     endpoint?: {uri: string, loopback_only: bool}
     preconditions: [{id, kind: credential|os-consent|process|context|call|session-handle, probe?, human?: bool}]
     discovery: {authority: static|introspect|probe|explore, sources: [{probe, bytes, digest}]}
   capability:
     id: <app>.<noun>.<verb>
     address: {argv|aevt|method|app_intent|http: …}                      # native, verbatim
     description: {text, trust: untrusted}
     input_schema: JSONSchema (root additionalProperties:false; adapter-enforced)
     excluded_inputs?: [{name, reason}]                                   # e.g. --web, crosses effect class
     output_schema: JSONSchema | {kind: field-names, fields:[…]} | {kind: unknown}
     effects: {<REA bit>: true|false|unknown}                             # 10 bits, src/contracts/toolEffects.ts:2-21
     effect_provenance: {<bit>: declared(src)|floor(table:key)|observed(probe)|inferred(why)|human(who)}
     branches?: [{when: {<input>: value}, effects: {…}, provenance}]      # flag/method-conditioned
     preconditions: [ref|inline]
     lifetime: one-shot | session
     oracle: {kind: self|read-back|negative|event|cross-surface|none, capability?, fields?: {in→out}, status}
     dry_run?: {address, kind}
     family?: string
     status: proposed | scaffolded | fixture-verified | live-verified | withheld
     withheld_reason?: mixed-authority | not-invokable | no-oracle | unknown-effect | …
     next_probes?: [{operation, rationale, confidence}]
   ```

   Level, MCP annotations, title, CLI flags and skill text are all *derived* from this record.
2. **Tri-state effect bits plus a narrowing log.** Policy maps `unknown → true`. Lowering a bit requires a provenance entry (`observed`, `human`). This replaces R4's all-true booleans without loosening safety.
3. **Ship floor tables as versioned data per family.** Use the max-combine rule from discovery §4.3, and run every table output through a review queue:
   - the Cobra verb lexicon;
   - the Standard Suite table;
   - the CDP verb and precondition table;
   - the App Intents protocol and schema table.

   The pilot found table errors at roughly a 5-10% rate on gh and CDP (INFERRED from the manual-review counts).
4. **Branch extraction as a deterministic lint.** Flags named `--force`, `--yes`, `--delete-*`, `--web`, `-X/--method`, `--clobber`, `--hard` create `branches[]` entries or `excluded_inputs`. A tool is never admitted with an unresolved mixed branch (R3).
5. **Two self-descriptions per target, diffed.** Use Cobra `reference` vs `__complete`, `/json/protocol` vs `Schema.getDomains`, and sdef vs App Intents. A disagreement is a finding attached to identity, not noise.
6. **Probes the kit should run by default on a fixture lane** (each was cheap and found something here):
   - parser-conformance: an unknown flag or parameter and a wrong type;
   - `observed ⊆ declared` by file-tree snapshot of the fixture HOME;
   - confirmed-write by read-back;
   - a **bogus-id negative control** for deletes and removes (CDP returned success for a nonexistent breakpoint).
7. **The oracle is a typed object with `kind`, and field maps are first-class work items.** Capability-level pairing by noun is deterministic (98% of gh writes found a candidate). Field-level maps are an LLM-proposal plus probe-confirmation queue, because only 21% name-matched. Writes without a confirmed map ship `withheld: no-oracle`, as REA's confirmed-write admission does.
8. **Model OS consent as a precondition with `human: true`.** Examples are TCC Automation per (caller, app), and Shortcuts or Siri reachability for App Intents (`isDiscoverable=false` → `withheld: not-invokable`). The kit cannot verify Apple-Event surfaces unattended. A macOS live lane needs a pre-granted VM or a disposable user, plus a non-user library.
9. **Runtime-root hygiene for sandbox lanes.** Keep fixture profile paths short, because Electron's unix-socket path limit is 103 characters. Do not wrap Chromium or Electron in `sandbox-exec`, because the nested Seatbelt fails. Use network-deny at another layer, or none plus a telemetry-off profile. For CLIs, `env -i` with a scratch HOME plus `sandbox-exec` deny-network worked.
10. **Close schemas in the adapter, always.** At least one real protocol (CDP) silently ignores unknown parameters. "A misspelled parameter never degrades into an omitted one" (REA, as summarized in `rea-desktop-applications-and-agent-environments.md` §8.2) cannot be delegated to the target.
11. **Report "scaffolded in minutes" and "audited" as different states.** Emit per-target counters for unknown bits, unresolved branches and writes without an oracle. This makes the remaining human work measurable, extending walkthrough #9's counters.

## Open questions

1. **Specifier schema.** What JSON Schema form should AppleScript object specifiers (and similar query-typed parameters) take? Options are a constrained path grammar, an id-only reference (`persistent ID`), or a separate `resolve` capability that returns ids. Picking one decides whether sdef targets are "typed" in practice.
2. **App Intents primitive codes.** Is the `typeIdentifier` → type table (0, 1, 12, …) documented or stable across `toolsVersion`? UNKNOWN. It was inferred here from parameter names only.
3. **Access-groups as effect evidence.** Are sdef `access-group` identifiers a reliable effect proxy across apps? Music's naming (`library.read`, `library.read-write`, `playback`, `user-interface`) is suggestive. Apple documents them as Apple-Event access scoping, not effect classes.
4. **Authoritative CDP catalog.** When `/json/protocol` and `Schema.getDomains` disagree (14 vs 5 domains), which one should admit a capability? Should commands outside the runtime list be withheld until probed?
5. **Negative oracles for remote deletes (gh).** Can the not-found signal be learned from one fixture-repo probe per noun? Or must the kit require a target-specific "absent" predicate written by a human?
6. **Who maps fields for oracles at scale?** For gh (239 unmatched write flags), is an LLM proposal plus one confirming fixture probe per field acceptable evidence for `fixture-verified`? Or does each map need human sign-off?
7. **macOS live lane.** What is the cheapest isolated environment with pre-granted TCC Automation and a throwaway Music or Notes library: a VM image, a second login user, or MDM PPPC profiles? Without one, sdef and App Intents capabilities stay `proposed` forever.
8. **Electron targets.** With Chromium unwrappable by `sandbox-exec`, what network-deny mechanism should the kit use for exploration lanes: a per-process firewall such as a NetworkExtension, a VM, or `--proxy-server` to a deny proxy?
9. **Positional grammars.** Is it worth parsing Cobra `USAGE` grammars (`[<a> | <b>]`, `<x>...`) deterministically? Or is this the right place for the LLM-plus-parser-probe loop in discovery §5?
10. **Not piloted.** An OpenAPI/GraphQL service and an Electron app's full browser protocol (blocked here by the issues in §1) remain untested end to end. HTTP-method semantics would give declared effects there, which may change the "declared effects ≈ 0" conclusion for web-API targets.
