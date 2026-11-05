# Slice 1 acceptance run (build-plan §4)

The §4 acceptance demo, run by hand on this Mac on 2026-11-05 after integrating WP0-WP6. The run used:
- macOS arm64, git 2.48.1 (Homebrew), gh 2.102.0, Homebrew Python 3.14.7, LibreOffice 26.8.0 (cask);
- the release binaries `target/integ/release/{aad,aa}` (`CARGO_TARGET_DIR=target/integ cargo build --release`);
- a scratch workspace and runtime dir: `AA_HOME=target/demo/home`, `AA_RUNTIME_DIR=target/demo/rt-parent/rt`, with
  `examples/policy.toml` installed 0600 as `$AA_HOME/policy.toml`;
- `aad --policy $AA_HOME/policy.toml --workspace $AA_HOME --runtime-dir $AA_RUNTIME_DIR`, started in the background.

Nothing under `~/.config` or `~/.local` was created or read for engine state. Output is trimmed with `jq` and line
cuts. The repo is `R=$(mktemp -d)/repo`, as in §4. `examples/policy.toml` covers it through `read_roots`
(`/var/folders`).

## Results

| # | §4 step | Expected | Observed | Result |
|---|---|---|---|---|
| 1 | `aa status` | caller_role AGENT, halted=false, confinement_available=true, assurance SAME_PRINCIPAL | `ROLE_AGENT`, `halt:{}`, `confinementAvailable:true`. `assurance` and `halted` are omitted because they are proto3 zero values (`ASSURANCE_TIER_SAME_PRINCIPAL`, false). | PASS |
| 2 | `aa target add git …` | registered, identity pinned | exit 0; `/opt/homebrew/Cellar/git/2.48.1/bin/git` pinned (sha256); overlay installed | PASS |
| 3 | `aa discover git` | 5 native + 2 overlay mode-split candidates; unknown bits listed per capability | `candidates 7`, `harvestedCandidates 5`, `unknownBits 7`; each capability's `EFFECTS_UNKNOWN` hint lists the undecided fields; `git.branch` and `git.rev-parse` are `MIXED_AUTHORITY` | PASS |
| 4 | `aa verify git` | L0/L1/L3; git.status, git.log, git.branch.list SERVABLE (READ, CONFINED) | exactly those 3 are servable; `describe git.status` gives `EFFECT_LEVEL_READ`, `ADMISSION_STATE_SERVABLE`, `ISOLATION_CONFINED` | PASS |
| 5 | `aa list git` | git.add WITHHELD NOT_VERIFIED; git.branch.delete WITHHELD; git.rev-parse MIXED_AUTHORITY | git.add: `NOT_VERIFIED, EFFECTS_UNKNOWN, NO_ORACLE`; git.branch.delete: same; git.rev-parse and git.branch: `MIXED_AUTHORITY` | PASS |
| 6 | `aa x git.status …` | exit 0; porcelain v2 lines | exit 0; `# branch.oid …`, `# branch.head master` | PASS |
| 6b | `aa x git.log … max_count:1` (extra) | §7.3 `--max-count` served | exit 0; one tab-separated commit line | PASS |
| 7 | `aa x git.add …` | 3: withheld, nothing ran | exit 3, `ERROR_CODE_CAPABILITY_WITHHELD`; ActionRecord `OUTCOME_NOT_DISPATCHED` | PASS |
| 7b | `aa x git.status` on `/etc` / `{"repo":1}` (extra, from §3.6) | 3 `OUTSIDE_PATH_SCOPE` / 2 | exit 3 `DENIAL_REASON_OUTSIDE_PATH_SCOPE` / exit 2 `INVALID_REQUEST` | PASS |
| 8 | `aa halt --reason demo && aa x git.status …` | 4: halted | halt epoch 1; invoke exit 4 `ERROR_CODE_ENGINE_HALTED`; the refused attempt is audited (seq 36) | PASS |
| 9 | `aa unhalt --epoch $(aa status --json \| jq .halt.haltEpoch)` | unhalted | exit 0; jq prints the uint64 epoch as the quoted string `"1"`, which `aa unhalt` accepts; the next invoke succeeds | PASS |
| 10 | `aa audit --last 12 --verify` | intents for pipeline actions; one ActionRecord per invoke; chain intact | `intact:true`, `headMatches:true`; one ActionRecord per dispatched or refused-by-gate invoke; intents for `aa.pipeline.harvest.*`, `verify.l1` and `verify.l3`. The last 12 entries contain only the invokes, so pipeline intents are shown with a wider query. | PASS |
| 11 | `aa import cli-anything … --harness libreoffice --python <venv>` | imported, install_cmd stored, never run | Import succeeded (STATIC). It pinned the interpreter, its framework `Python.app` companion, the entry module and `backend:soffice`, plus precondition `binary:soffice`. The python given was `target/integ-venv` (click and the notes fixture only), because no LibreOffice harness venv exists. | PARTIAL |
| 12 | `aa discover/verify cli-anything.libreoffice`, `aa list … --servable` | 40 candidates; document.info, writer.list … SERVABLE | **Not run to completion.** discover fails cleanly (exit 1, `UPSTREAM_ERROR entry_point_not_found`, remediation "install the harness into the pinned interpreter's venv (operator step)"), because the venv has no harness. In this session the auto-mode permission classifier refused installing the harness into a scratch venv ("Code from External"), the same refusal WP3 hit. | NOT RUN |
| 13 | SIGTERM to `aad` (extra) | clean shutdown | aad exited and removed both sockets | PASS |

To finish steps 11-12, an operator pre-installs the harness (§4 comment) and re-runs. Install from a copy so the
read-only checkout is not written to:

```sh
cp -R /Users/nhanclie/CLI-Anything/libreoffice/agent-harness /tmp/lo-src
uv venv ~/.local/share/aa-venvs/lo && uv pip install --python ~/.local/share/aa-venvs/lo/bin/python /tmp/lo-src
aa import cli-anything /Users/nhanclie/CLI-Anything --harness libreoffice --python ~/.local/share/aa-venvs/lo/bin/python \
   --authorization "owner workstation; MPL-2.0 app; Apache-2.0 harness"
aa discover cli-anything.libreoffice && aa verify cli-anything.libreoffice && aa list cli-anything.libreoffice --servable
# and the opt-in e2e:
AA_TEST_CLI_ANYTHING=/Users/nhanclie/CLI-Anything AA_TEST_LO_PYTHON=~/.local/share/aa-venvs/lo/bin/python \
   cargo nextest run -p aa-cli --test e2e_libreoffice
```

## Gates (definition of done, §1)

| Gate | Command | Result |
|---|---|---|
| 1 | `CARGO_TARGET_DIR=target/integ scripts/check.sh` (buf lint, buf format, fmt, clippy `-D warnings`, nextest) | green: 309 tests run, 309 passed, 0 skipped (`AA_TEST_PYTHON` picked up from `target/integ-venv`; `e2e_libreoffice` passes only by printing SKIP) |
| 2 | §4 demo above | git half: PASS; CLI-Anything half: blocked on the operator-installed harness venv |
| 3 | `crates/aa-cli/tests/e2e_*.rs` against a real `aad` | `e2e_git` (3 tests), `e2e_gh`, `e2e_click` (with `AA_TEST_PYTHON=target/integ-venv/bin/python`, Homebrew framework Python): PASS. `e2e_libreoffice`: opt-in, prints SKIP without its env vars |

## Transcript

```text
$ aa status
{"agentListen":"unix:///Users/nhanclie/agentic-anything/target/demo/rt-parent/rt/agent.sock","auditHeadSha256":"0c13ae477e0c9be1dd994703cb5573a499d65b94557c6d7b218e26ff4d656453","callerRole":"ROLE_AGENT","confinementAvailable":true,"engineVersion":"0.1.0","halt":{},"operatorListen":"unix:///Users/nhanclie/agentic-anything/target/demo/rt-parent/rt/operator.sock","protocolVersion":"agentic_anything.
# exit 0

# R=/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo

$ aa target add git --exec git --overlay targets/git/target.aa.yaml --authorization 'owner workstation' | jq -c '{overlayInstalled, id: .target.id, files: [.target.identity.files[] | {role, realpath}]}'
{"overlayInstalled":"/Users/nhanclie/agentic-anything/target/demo/home/targets/git/target.aa.yaml","id":"git","files":[{"role":"executable","realpath":"/opt/homebrew/Cellar/git/2.48.1/bin/git"}]}
# exit 0

$ aa discover git | jq -c '.counts'
{"candidates":7,"excluded":0,"harvestedCandidates":5,"servable":0,"unknownBits":7,"unresolvedBranches":2,"withheld":7,"withheldByReason":{"EFFECTS_UNKNOWN":7,"MIXED_AUTHORITY":3,"NOT_VERIFIED":21},"writesWithoutOracle":0}
# exit 0

$ aa discover git | jq -c '.capabilities[] | {id, withheld: ([.withheld[]?.reason | sub("WITHHELD_REASON_"; "")] | unique), undecided: ([.withheld[]? | select(.reason == "WITHHELD_REASON_EFFECTS_UNKNOWN") | .hint][0] // "" | .[0:120])}'
{"id":"git.add","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED"],"undecided":"undecided: writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_code, mutates_devic"}
{"id":"git.branch","withheld":["EFFECTS_UNKNOWN","MIXED_AUTHORITY","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_cod"}
{"id":"git.branch.delete","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, executes_foreign_code, mutates_device,"}
{"id":"git.branch.list","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_cod"}
{"id":"git.log","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_cod"}
{"id":"git.rev-parse","withheld":["EFFECTS_UNKNOWN","MIXED_AUTHORITY","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_cod"}
{"id":"git.status","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED"],"undecided":"undecided: mutates_target, writes_filesystem, launches_process, accesses_network, may_discard_data, executes_foreign_cod"}
# exit 0

$ aa verify git | jq -c '{progress: (.progress // null | length?), servable: [.capabilities[]? | select((.withheld // []) | length == 0) | .id]}'
{"progress":0,"servable":["git.branch.list","git.log","git.status"]}
# exit 0

$ aa list git | jq -c '.capabilities[] | {id, withheld: ([.withheld[]?.reason | sub("WITHHELD_REASON_"; "")] | unique)}'
{"id":"git.add","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED","NO_ORACLE"]}
{"id":"git.branch","withheld":["EFFECTS_UNKNOWN","MIXED_AUTHORITY","NOT_VERIFIED"]}
{"id":"git.branch.delete","withheld":["EFFECTS_UNKNOWN","NOT_VERIFIED","NO_ORACLE"]}
{"id":"git.branch.list","withheld":[]}
{"id":"git.log","withheld":[]}
{"id":"git.rev-parse","withheld":["EFFECTS_UNKNOWN","MIXED_AUTHORITY","NOT_VERIFIED"]}
{"id":"git.status","withheld":[]}
# exit 0

$ aa list git --servable | jq -c '[.capabilities[].id]'
["git.branch.list","git.log","git.status"]
# exit 0

$ aa describe git.status | jq -c '{level: .capability.level, admission: .capability.admission.state, isolation: .capability.admission.requiredRuntimeIsolation}'
{"level":"EFFECT_LEVEL_READ","admission":"ADMISSION_STATE_SERVABLE","isolation":"ISOLATION_CONFINED"}
# exit 0

$ aa x git.status --input-json '{"repo":"/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo"}'
{"lines":["# branch.oid 0598efbd6a56da0c3e2abbe863a93e059ce5a9af","# branch.head master"],"text":"# branch.oid 0598efbd6a56da0c3e2abbe863a93e059ce5a9af\n# branch.head master\n"}
# exit 0

$ aa x git.log --input-json '{"repo":"/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo","max_count":1}'
{"lines":["0598efbd6a56da0c3e2abbe863a93e059ce5a9af\tme\t2026-11-05T23:19:22+11:00\tinit"],"text":"0598efbd6a56da0c3e2abbe863a93e059ce5a9af\tme\t2026-11-05T23:19:22+11:00\tinit\n"}
# exit 0

$ aa x git.add --input-json '{"repo":"/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo","pathspec":["README"]}' 2>&1 | jq -c '.error | {code, message: .message[0:100]}'; echo "exit=${PIPESTATUS[0]}"
{"code":"ERROR_CODE_CAPABILITY_WITHHELD","message":"git.add is withheld"}
exit=3
# exit 0

$ aa x git.status --input-json '{"repo":"/etc"}' 2>&1 | jq -c '.error | {code, denialReason}'; echo "exit=${PIPESTATUS[0]}"
{"code":"ERROR_CODE_POLICY_DENIED","denialReason":"DENIAL_REASON_OUTSIDE_PATH_SCOPE"}
exit=3
# exit 0

$ aa x git.status --input-json '{"repo":1}' 2>&1 | jq -c '.error | {code, message: .message[0:100]}'; echo "exit=${PIPESTATUS[0]}"
{"code":"ERROR_CODE_INVALID_REQUEST","message":"input rejected by the schema of git.status: at '/repo': 1 is not of type \"string\""}
exit=2
# exit 0

$ aa halt --reason demo && aa x git.status --input-json '{"repo":"/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo"}'; echo "exit=$?"
{"halt":{"haltEpoch":"1","halted":true,"reason":"demo","source":"HALT_SOURCE_RPC"}}
{"error":{"category":"ERROR_CATEGORY_SAFETY_STOP","code":"ERROR_CODE_ENGINE_HALTED","message":"the engine is halted (epoch 1); /agentic_anything.v1.RuntimeService/Invoke is refused","recovery":{"kind":"RECOVERY_KIND_RESTART_ENGINE","operation":"aa unhalt"},"remediation":"operator: aa unhalt --epoch 1"}}
exit=4
# exit 0

$ aa unhalt --epoch $(aa status --json | jq .halt.haltEpoch)
{"halt":{"haltEpoch":"1"}}
# exit 0

$ aa status | jq -c '{callerRole, halt, confinementAvailable, assurance: (.assurance // "ASSURANCE_TIER_SAME_PRINCIPAL (proto3 zero value, omitted)")}'
{"callerRole":"ROLE_AGENT","halt":{"haltEpoch":"1"},"confinementAvailable":true,"assurance":"ASSURANCE_TIER_SAME_PRINCIPAL (proto3 zero value, omitted)"}
# exit 0

$ aa x git.status --input-json '{"repo":"/var/folders/_1/xkyryw0d1szg1hjd3_7k9l780000gq/T/tmp.W5IGjnLuhE/repo"}' | jq -c '.lines'
["# branch.oid 0598efbd6a56da0c3e2abbe863a93e059ce5a9af","# branch.head master"]
# exit 0

$ aa audit --last 12 --verify | jq -c '.chain, (.entries[] | {seq, kind: (if .intent then "intent" elif .action then "action" elif .admin then "admin" else "other" end), cap: (.intent.capabilityId // .action.capabilityId // .admin.subject), adminKind: .admin.kind, outcome: .action.outcome, code: .action.error.code})'
{"entries":"38","headMatches":true,"headSha256":"95d3574d3944babdcd743c8d262e0e60eea30c9c711bb716e21c746542977158","intact":true}
{"seq":"27","kind":"admin","cap":"git.branch.list","adminKind":"ADMIN_EVENT_KIND_ADMISSION_CHANGED","outcome":null,"code":null}
{"seq":"28","kind":"admin","cap":"git.log","adminKind":"ADMIN_EVENT_KIND_ADMISSION_CHANGED","outcome":null,"code":null}
{"seq":"29","kind":"admin","cap":"git.status","adminKind":"ADMIN_EVENT_KIND_ADMISSION_CHANGED","outcome":null,"code":null}
{"seq":"30","kind":"action","cap":"git.status","adminKind":null,"outcome":"OUTCOME_COMPLETED","code":null}
{"seq":"31","kind":"action","cap":"git.log","adminKind":null,"outcome":"OUTCOME_COMPLETED","code":null}
{"seq":"32","kind":"action","cap":"git.add","adminKind":null,"outcome":"OUTCOME_NOT_DISPATCHED","code":"ERROR_CODE_CAPABILITY_WITHHELD"}
{"seq":"33","kind":"action","cap":"git.status","adminKind":null,"outcome":"OUTCOME_NOT_DISPATCHED","code":"ERROR_CODE_POLICY_DENIED"}
{"seq":"34","kind":"action","cap":"git.status","adminKind":null,"outcome":"OUTCOME_NOT_DISPATCHED","code":"ERROR_CODE_INVALID_REQUEST"}
{"seq":"35","kind":"admin","cap":"engine","adminKind":"ADMIN_EVENT_KIND_HALT","outcome":null,"code":null}
{"seq":"36","kind":"admin","cap":"/agentic_anything.v1.RuntimeService/Invoke","adminKind":null,"outcome":null,"code":null}
{"seq":"37","kind":"admin","cap":"engine","adminKind":"ADMIN_EVENT_KIND_UNHALT","outcome":null,"code":null}
{"seq":"38","kind":"action","cap":"git.status","adminKind":null,"outcome":"OUTCOME_COMPLETED","code":null}
# exit 0

$ aa import cli-anything /Users/nhanclie/CLI-Anything --harness libreoffice --python /Users/nhanclie/agentic-anything/target/integ-venv/bin/python --authorization 'owner workstation; MPL-2.0 app; Apache-2.0 harness' | jq -c '.harnesses[]? | {name, targetId, error: (.error | {code, message: .message[0:160], remediation: .remediation[0:200]})}'
{"name":"libreoffice","targetId":"cli-anything.libreoffice","error":{"code":null,"message":null,"remediation":null}}
# exit 0

$ aa target show cli-anything.libreoffice | jq -c '{id: .target.id, kind: .target.kind, files: [.target.identity.files[]? | {role, realpath}], preconditions: [.target.preconditions[]?.id]}'
{"id":"cli-anything.libreoffice","kind":"TARGET_KIND_CLI_ANYTHING_HARNESS","files":[{"role":"interpreter","realpath":"/Users/nhanclie/agentic-anything/target/integ-venv/bin/python"},{"role":"interpreter:app","realpath":"/opt/homebrew/Cellar/python@3.14/3.14.7/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python"},{"role":"entry_module","realpath":"/Users/nhanclie/CL …
# exit 0

$ aa discover cli-anything.libreoffice
{"error":{"category":"ERROR_CATEGORY_EXECUTION","code":"ERROR_CODE_UPSTREAM_ERROR","detailsJson":"{\"entry_point\":\"cli-anything-libreoffice\",\"error\":\"entry_point_not_found\",\"protocol\":\"aa-click-info/1\"}","message":"click-info@1: shim failed for entry point 'cli-anything-libreoffice' (exit Some(66)): entry_point_not_found","remediation":"install the harness into the pinned interpreter's  …
# exit 1

$ aa audit --last 60 | jq -c '[.entries[] | select(.intent) | .intent.capabilityId] | group_by(.) | map({(.[0]): length}) | add'   # pipeline intents
{"aa.pipeline.harvest.click-info":2,"aa.pipeline.harvest.help-text":2,"aa.pipeline.verify.l1":3,"aa.pipeline.verify.l3":3}
# exit 0

$ aa audit --last 60 | jq -c '[.entries[] | select(.action) | .action.capabilityId] | group_by(.) | map({(.[0]): length}) | add'   # action records
{"aa.pipeline.harvest.click-info":2,"aa.pipeline.harvest.help-text":2,"aa.pipeline.import.cli-anything":1,"aa.pipeline.verify.l0":7,"aa.pipeline.verify.l1":3,"aa.pipeline.verify.l3":3,"git.add":1,"git.log":1,"git.status":4}
# exit 0

$ aa audit --verify | jq -c .chain
{"entries":"44","headMatches":true,"headSha256":"d1df02217acb3041d942fb8ff4931dd9082f827b800a3c1a17841c0d1490958f","intact":true}
# exit 0
```
