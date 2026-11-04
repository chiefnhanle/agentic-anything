# REA: setup, distribution, supply chain, and licensing

Research area: `setup-distribution-supply-chain-licensing`
Source checkout: `scratchpad/rea-src` (morluto/rea, package `rea-agents@6.8.0`, shallow clone at commit `52ff312`, 2026-11-04). Paths below are relative to that checkout unless prefixed otherwise. OBSERVED = read in source; INFER = my interpretation.

## Summary

- One artifact, many front doors (OBSERVED): the npm tarball `rea-agents` is simultaneously the CLI (`bin.rea`), the stdio/HTTP MCP server (`scripts/rea.mjs mcp`), the Claude Code plugin (`.claude-plugin/plugin.json`), the Codex plugin (`.codex-plugin/plugin.json` + `mcp.json`), the MCP Registry entry (`server.json`), and the skills.sh skill source (`.agents/skills/reverse-engineer-anything`). Release Please rewrites the version pin in all of them in one PR (`release-please-config.json:19-65`).
- `install.sh` is a hardened `curl | bash` wrapper, not a package installer (OBSERVED): brace-group truncation guard, `set -euo pipefail`, exact-semver resolution from the GitHub Releases API, refuses build-metadata versions, `npm install --global` into `~/.local` on Linux, then verifies `rea --version` equals the requested version before optionally starting `rea setup` on `/dev/tty` (`install.sh:2-9,130-174`).
- `rea setup` is a plan/approve/transact flow (OBSERVED): discover clients without mutation (`SetupPlan.ts:51-150`), render a preflight listing target path, backup path, network origins, integrity evidence, and exact shell commands per action (`src/cli/interactiveSetup.ts:287-309`), require explicit consent (`--yes` for unattended, a separate `--install-hopper` for the external download), then write client config via staged publication + content-addressed `.rea.backup` / `.rea.backup.<sha256>.recovery` + readback + rollback (`SetupClientConfiguration.ts:365-445`, `SetupRecoveryBackup.ts`).
- Setup registers 25 agent clients by editing their own config files additively (OBSERVED, `src/application/SupportedClients.ts:524-760`, `docs/installation.md:628-660`): Claude Code, Claude Desktop, Codex, Cursor, Gemini CLI, Windsurf, Devin, OpenCode, Antigravity, Copilot CLI, Command Code, Qwen Code, Kimi, VS Code, Grok Build, OMP, Pi, Hermes, Qoder, Factory Droid, Trae, WorkBuddy, ZCode, DeepSeek Harness, FreeBuff. The registered command is `<absolute runtime> <absolute entry script> mcp`, never `npx` (`ClientRegistrationIdentity.ts:64-87`).
- Hopper is the only third-party tool REA will download, and it does so with pinned publisher identity (OBSERVED): macOS = vendor HTTPS feed + SHA-1/length match + `hdiutil` mount + `codesign --verify --deep --strict` + Developer ID requirement pinned to team `2AMA2753NF` (`src/domain/hopperPublisher.ts`, `MacHopper.ts:90-101,440-500`, `MacCodeSignature.ts`); Linux = detached PGP signature verified with `openpgp` against a pinned primary fingerprint `7C2D73BA...1BCD` (`DetachedPgpSignature.ts`, `HopperReleaseFeed.ts:106`, `LinuxHopper.ts:65-71`), then `apt-get`/`dnf`/`pacman` via `pkexec`, never `sudo`.
- Everything else is bring-your-own (OBSERVED): Ghidra, IDA (+ida-pro-mcp / ida-free-mcp), JEB, Kuna, JADX, Binwalk, Unblob, WABT, pwntools, mitmproxy, adb. REA "never downloads engines during an analysis operation" (`third_party/README.md:23-28`), records digests of caller-supplied executables, and refuses to attribute modified bytes to an audited release.
- Dependency hygiene (OBSERVED): every `dependencies`/`devDependencies` entry is an exact version (zero `^`/`~`; verified with a script over `package.json`); `.npmrc` = `engine-strict=true`; `.nvmrc` + `packageManager: npm@11.16.0`; `package-lock.json` plus a generated runtime-only `npm-shrinkwrap.json` in the tarball; `licenses/npm-dependencies.txt` enumerates every lockfile entry with resolved URL + integrity and a runtime fingerprint checked in CI (`scripts/generate-third-party-inventory.mjs`).
- Release pipeline is reviewed-SHA-bound and fail-closed (OBSERVED, `docs/releasing.md`, `.github/workflows/release.yml:405-440`): qualification builds one tarball; rehearsal installs it with `--ignore-scripts` into an empty prefix and drives the real MCP catalog; the publisher job has only `contents: read, id-token: write`, downloads the same artifact, validates source/run/attempt/version/Node/digest with Node built-ins, and runs `npm publish <tgz> --ignore-scripts`. No `NPM_TOKEN`/`NODE_AUTH_TOKEN` appears anywhere in workflows (INFER: OIDC trusted publishing, which yields npm provenance). All GitHub Actions are pinned by commit SHA.
- `rea update` is a verified self-update (OBSERVED, `src/application/Update.ts:90-118,228-243`): detects the owning npm/bun global install, resolves `dist-tags.latest`, runs `npm install --global --ignore-scripts --prefix <owner> rea-agents@<exact>`, re-executes the binary and refuses success unless the reported version equals the target; never touches npx cache copies or source checkouts.
- Licensing posture (OBSERVED): MIT for REA code; `THIRD_PARTY_NOTICES.md` and `DISCLAIMER.md` state REA grants no license/key/entitlement for IDA/Hex-Rays, Hopper, JEB; vendor EULAs were "checked on 2026-10-12"; JEB's EULA restricts disclosure of decompiled output; ida-pro-mcp explicitly does not support IDA Free. `CONTRIBUTING.md:137-200` excludes DRM/license/anti-cheat bypass, packer defeat, and attack automation, and the "fail-closed integration" rule says unresolved boundary questions block merge.

## How it works

### Acquisition paths

| Path | Where | Notes (OBSERVED) |
| --- | --- | --- |
| `npx rea-agents setup` | `README.md:27,83` | Primary quick start. README warns: "Run package acquisition from a trusted directory outside the target project" because npx can resolve project-local packages/config before REA's own code runs (`docs/installation.md:1799-1803`). |
| `curl ... install.sh \| bash` | `install.sh` | Resolves latest `rea-agents-X.Y.Z` tag via `api.github.com` with `--proto '=https' --tlsv1.2`; validates tag with a full SemVer regex; `npm install --global rea-agents@X.Y.Z`; verifies installed `--version` string equals requested; launches `rea setup` only when `/dev/tty` and stdout/stderr are TTYs. |
| `npm install --global rea-agents` then `rea setup` | `README.md:130-135`, `docs/installation.md:1848-1863` | Recommended for many concurrent agents (avoids one npx launcher per MCP session). |
| Claude Code plugin | `.claude-plugin/plugin.json`, `marketplace.json` | `/plugin marketplace add morluto/rea`, `/plugin install rea@rea`. Marketplace `source: npm, package: rea-agents, version: 6.8.0` (exact). Plugin `mcpServers.rea` = `node ${CLAUDE_PLUGIN_ROOT}/scripts/plugin-mcp-launcher.mjs 6.8.0`. |
| Codex plugin | `.codex-plugin/plugin.json`, `mcp.json`, `.agents/plugins/marketplace.json` | Same launcher with `cwd: "."` and `startup_timeout_sec: 30`. |
| MCP Registry | `server.json` | `io.github.morluto/rea`, `runtimeHint: npx`, positional arg `mcp`, registry base `https://registry.npmjs.org`. |
| skills.sh skill-only | `README.md:15,371-375`, `docs/installation.md:598-625` | `npx skills add morluto/rea --skill reverse-engineer-anything` installs instructions only. `scripts/verify-repository-skill.mjs` checks that the Skills CLI installs exactly the authored bundle byte-for-byte using a separately locked Skills CLI (`scripts/verify-tools/skills/package-lock.json`). |

Official-source warning (OBSERVED, `README.md:31-35`): "Get REA from morluto/rea, rea.tools, or the rea-agents npm package... A similar repository name or copied README does not establish that a download is official."

### `scripts/rea.mjs` dispatcher

- OBSERVED (`scripts/rea.mjs:12-60`): reads `package.json` version, prepares process env, then checks that the compiled `dist/` runtime files exist before importing anything; missing runtime prints a remediation and exits 1 rather than falling back. Routes `mcp` (stdio), `mcp --http`, `mcp doctor`, and the Incur-based CLI. Handles EPIPE/SIGPIPE exit 141 explicitly.
- `scripts/plugin-mcp-launcher.mjs` (OBSERVED, lines 161-208 in the concatenated read): takes exactly one `X.Y.Z` pin, refuses if `package.json.version` differs ("reinstall the plugin"), checks `dist/mcp/main.js` exists, then rewrites `process.argv` and imports `rea.mjs` in-process. Comment: "without another npx child." The release checkpoint validator asserts the literal launcher paths and pin positions in both plugin manifests (`scripts/lib/release-checkpoint.mjs:7-31`).

### `rea setup` flow

1. Preflight / discovery (OBSERVED, `src/application/SetupPlan.ts:51-150`): runs doctor, decides Hopper eligibility per platform, lists supported vs detected clients, classifies each existing `rea` registration as `owned` / foreign / unknown by comparing the launcher path with `CURRENT_ENTRYPOINT_PATH`. Newly detected clients are shown but unselected: "detection gives setup context, not permission" (`docs/installation.md:38-43`).
2. Plan rendering (OBSERVED, `src/cli/interactiveSetup.ts:287-309`): for each `SetupAction` (`kind: install_hopper | configure_client | install_skill`, `operation: create|update|install`) prints target, `backup:`, `network:` origins, `integrity:` evidence, `command:` lines, and an `external software` marker; ends with "REA will preserve unrelated configuration and will not install or upgrade Node.js, npm, Homebrew, Java, or Ghidra." `--dry-run` prints the same plan and exits; `--json` with dry-run yields a machine-readable plan (`setupCommands.ts:92-95,245-249`).
3. Consent (OBSERVED, `setupCommands.ts:54-61,272-277`): `--yes` approves user-owned actions; Hopper needs an additional `--install-hopper`; interactive flow asks a separate "Install Hopper for deep binary analysis?" (`interactiveSetup.ts:151-160`). Ctrl-C = "Setup cancelled. No changes were made."
4. Client config transaction (OBSERVED, `SetupClientConfiguration.ts:290-445`, `docs/installation.md:60-96`): parse existing JSON/JSONC/TOML/YAML preserving comments; additive merge `{...fallbackServers, ...parsed.servers, rea: desired}`; refuse if the target file changed since planning (`target_changed`, `conflict`); refuse symlinked or foreign-owned paths; `preserveCurrentRecoveryBackup` keeps the first `.rea.backup` and, if the current bytes differ, writes a content-addressed `.rea.backup.<sha256>.recovery` with `COPYFILE_EXCL`; staged write in a private directory then publish; first-run uses an exclusive hard link; readback must parse and contain the desired entry or it rolls back. Idempotence: an aligned owned registration reports `unchanged` / "Already current" (`SetupClients.ts:93-96`). Foreign `rea` entries block setup rather than being overwritten.
5. Skill install (OBSERVED, `SetupSkill.ts:30-41,184-281`): copies the generated `skills/reverse-engineer-anything/` bundle (SKILL.md + 7 references) into each client's skill dir; ownership is proven by the SKILL.md front-matter `name:` matching `PRODUCT_IDENTITY.skillName`; same backup/rollback machinery.
6. Registration command (OBSERVED, `ClientRegistrationIdentity.ts:77-86`): `[absolute node-or-bun path, absolute entry script, "mcp"]`. Consequence documented at `docs/installation.md:1860-1863`: if setup ran via npx the registration points into npm's `_npx/<key>` cache, which must be retained.

### Hopper installer

- Vendor endpoints are constants (`HopperReleaseFeed.ts:18-23`): releases JSON, `hopper-public-key.asc`, and download prefix `https://www.hopperapp.com:443/downloader/public/`. `HopperDownload.ts` refuses non-HTTPS, credentialed URLs, redirects without Location, and >20 redirects; enforces a byte cap (`HOPPER_DOWNLOAD_BYTES`).
- macOS (`MacHopper.ts:200-260,440-500`): feed -> SHA-1 + length check (`hopperArchiveIntegrityMatches`) -> `hdiutil attach -readonly -nobrowse` -> locate `Hopper*.app` -> `codesign --verify --deep --strict` -> `codesign --verify --strict -R'= identifier "com.cryptic-apps.hopper-web-4" and anchor apple generic and certificate leaf[subject.OU] = "2AMA2753NF"'` -> compare `LSMinimumSystemVersion` -> `ditto` into a staging dir under `~/Applications` -> atomic rename, backing up any existing bundle. The team ID provenance is documented in a code comment with the DMG's SHA-1 (`src/domain/hopperPublisher.ts:1-8`).
- Linux (`LinuxHopper.ts`, `LinuxHopperPublisher.ts:148-265`): fetches the vendor public key (or uses a retained copy), checks key fingerprint equals the pinned primary, verifies a single detached binary signature over the whole package, records both primary and signing-subkey fingerprints, caches proofs keyed by sha256(bytes+key+sig), stages `release.json`, `public-key.asc`, `archive` for later launcher re-verification (installed launcher SHA-256 from the feed). Package install delegated to `apt-get install -y`, `dnf install -y`, or `pacman` via `pkexec` when not root; "REA never invokes sudo" (`SECURITY.md:46-48`).
- Discrepancy (OBSERVED): `SECURITY.md:46` still describes the Linux check as SHA-1 "corruption check, not a modern package signature", whereas the code and `linuxHopperInstallDisclosure` advertise detached PGP. Shallow clone prevents dating which is newer; INFER: SECURITY.md lags the code.
- Demo-mode automation (OBSERVED): `scripts/hopper-demo-x11.py` opens a private X display and clicks the "demo" button in Hopper's Registration dialog at fixed coordinates (`DEMO_CLICK_LEFT/BOTTOM`), with structured exit codes for unsupported dialogs. `docs/installation.md:1042-1073`: "REA does not read, install, or automate license credentials"; license activation is operator-owned. This is the closest REA gets to the licensing line: it automates choosing the vendor's own free demo mode, not bypassing it.

### Bring-your-own policy and commercial licensing

- `THIRD_PARTY_NOTICES.md:87-105` (Commercial tools and bridges): IDA Pro / Hex-Rays EULA; ida-pro-mcp MIT covers only the bridge and "explicitly states that IDA Free is unsupported"; Hopper license agreement distinguishes personal vs computer licenses, "REA setup may obtain the official installer after approval, but this grants no paid license"; JEB EULA "restricts public or third-party disclosure of decompiled code, reconstructed artifacts, and analysis reports unless expressly permitted by the license type." Vendor terms checked 2026-10-12.
- `docs/ida-provider.md:3,60,93`: "REA does not install, upgrade, activate, or license them"; IDA Free native plugin binds `0.0.0.0` and needs firewalling; IDA Free cloud decompiler "may request consent."
- `docs/kuna-provider.md:3`: Apache-2.0 `kuna` supplied by caller; "verification compares the reported version; it does not authenticate the executable"; executable digest retained in Evidence.
- `docs/jeb-analysis.md:3-20`: JEB "bring-your-own engine"; REA connects to a user-started JEB MCP at `127.0.0.1:8425` only (loopback-literal host enforced; redirects, remote hosts, credentials rejected); user must complete vendor EULA/key activation.
- `third_party/README.md`: submodules pin inspection sources (jadx-headless-mcp v0.7.1, binwalk v3.1.0, unblob 26.6.4, ghidra-nativeaot, wakaru) with commit hashes; "not included in the npm package"; fixtures downloaded by `scripts/fixtures/android/fetch.mjs` are SHA-256 pinned.
- `DISCLAIMER.md:10-19,80-86`: refuses help with DRM/license/anti-cheat circumvention, cheats, auth bypass, copying proprietary code, evading AI-provider policies, reselling AI access; "An open-source bridge or accessible API does not grant additional user, machine, feature, automation, hosting, or output-sharing rights."

### Supply-chain mechanics

- `package.json`: `engines.node: ^22.23.4 || ^24.22.1 || >=26.12.1` mirrored in `install.sh:60,123-126` (comment: "change both"); `files` whitelist includes `install.sh`, bridges, `skills`, `licenses`, both plugin manifests, `npm-shrinkwrap.json`; `prepublishOnly` verifies the Windows native artifact; `overrides` only for `qs` and `vite`.
- `scripts/prepack.mjs`: in a source tree runs `build:cached` + `generate-skill-metadata --check`; in a packed tree asserts the dist/skill files exist.
- `scripts/generate-skill-metadata.mjs:33-36`: rewrites `rea-agents@latest` to the exact pinned specifier in the shipped skill ("A shipped skill must run the version it ships with").
- `licenses/README.md:29-36`: notices collected by downloading locked archives, checking lockfile integrity, reading license files "without installing packages or executing lifecycle scripts."
- `.husky/pre-commit` = `npx --no-install lint-staged`; `pre-push` = `npm run check:fast`. Dependabot weekly for npm (root + skills verifier), pip (website), GitHub Actions, grouped minor/patch with 7-day cooldown (`.github/dependabot.yml`).
- `patches/release-please+17.3.0.patch` applied only by the release controller; "Version mismatch or a patch that neither applies cleanly nor matches an already-applied patch fails closed" (`docs/releasing.md:122-125`).
- `scripts/verify-release-checkpoint.mjs` + `scripts/lib/release-checkpoint.mjs`: validates that package.json, lockfile (both root versions), `.release-please-manifest.json`, `server.json`, plugin manifests, marketplaces, and CHANGELOG agree on the version and that plugin launcher literals are intact, and audits conventional-commit ancestry for breaking markers.
- `scripts/ci/prepare-npm-publication.mjs`: validates the qualified tarball envelope (source SHA, run/attempt, version, Node identity, digest) and compares to any already-published registry bytes/tag before publish; import graph is Node built-ins only.
- `scripts/verify-package-install.mjs:17-20,62-64`: installs with `--ignore-scripts --prefix <fresh dir>`.
- Versioning: `always-bump-minor` (`release-please-config.json:4`); docs warn a minor bump "does not promise backward compatibility."

### Security and reporting

- `SECURITY.md`: fixes only for latest version; private reporting via GitHub Security tab or email; per-session random capability tokens for Hopper/Ghidra bridges over current-user Unix sockets (POSIX) or DACL-restricted loopback TCP (Windows); "This is not a sandbox"; Ghidra runs in isolated temp project with private home/cache dirs; REA "does not download, install, upgrade, or modify Ghidra or Java."

## Design choices that make it extensible

- Client registry as data (`SupportedClients.ts`): each client is a record of `name`, `displayName`, `configPath` (static path or function of home/env/platform), optional `configPaths` fallbacks, optional `skillPath`, and a dialect for the parser/serializer. Adding an agent is a new row plus, if needed, a dialect.
- Comment/format-preserving config editing with a single additive merge point (`withClientServers`) and serializer that accepts the list of paths it is allowed to touch (`serializeClientConfiguration(..., allowedPaths, {removeShadowedMembersAt})`).
- `SetupAction` as the universal unit of consent: every side effect (file write, download, package install) is projected into one struct with `networkOrigins`, `commands`, `integrity`, `backupPath`, `external` so the renderer, JSON dry-run, and approval logic are shared.
- Host interfaces for every effectful step (`MacHopperInstallHost`, `MacHopperBundleInstallOperations`, `UpdateHost`, `SetupHost`): installers are pure state machines over injected effects, which is what lets the test suites (`Setup.*.test.ts`, `Update.test.ts`) cover failure/rollback paths.
- Ownership proof before mutation: `classifyClientRegistrationCommand` (owned/foreign/unknown) and `isManagedSkillManifest` gate edits so REA only ever rewrites what REA wrote.
- Publisher identity pinned in a tiny domain module (`src/domain/hopperPublisher.ts`, `HOPPER_LINUX_PRIMARY_FINGERPRINT`) separate from transport; verification primitives (`verifyDetachedPgpSignature`, `macDeveloperIdRequirement`) are reusable for any other vendor.
- One artifact, many manifests, one version authority: Release Please `extra-files` with jsonpath selectors keep plugin/marketplace/registry pins in sync; a validator refuses drift.
- Bring-your-own provider adapters record observed version + executable digest as Evidence rather than asserting trust.

## What to distill for an observe+drive control plane

Borrow:
- Plan/preview/approve/transact with per-action disclosure (`target`, `backup`, `network`, `integrity`, `command`, `external`) and a `--dry-run --json` that emits the same plan for an agent to review before `--yes`. This is directly applicable to a control plane that edits agent configs, installs drivers, or launches binaries.
- Additive, comment-preserving, ownership-gated config edits with content-addressed backups and readback verification; never overwrite a foreign entry under the same key.
- Register absolute runtime + absolute entry script in client configs; avoid `npx`-at-connect-time (slow, mutable, cache-dependent). If npx is unavoidable, document the cache-retention hazard as REA does.
- Exact-version everything: deps, `packageManager`, `.nvmrc`, `engine-strict`, `engines` floor duplicated in the shell installer, plugin pins rewritten by the release bot, shipped skill text pinned to its own version.
- Publish only an already-qualified, immutable tarball from a least-privilege OIDC job with `--ignore-scripts`; verify installed bytes against the registry afterwards; never republish/unpublish as a retry.
- Third-party downloads: pin the publisher (Apple team ID, PGP primary fingerprint) not just the hash from the same feed; enforce HTTPS across redirects; cap bytes; delegate privileged installs to the OS package manager through `pkexec` rather than `sudo`; show the exact commands first.
- Licensing docs pattern: one `THIRD_PARTY_NOTICES.md` table that separates "distributed", "bring-your-own OSS", and "commercial, you must license", with a dated "vendor terms checked on" line; `DISCLAIMER.md` out-of-scope list; CONTRIBUTING "fail-closed integration" rule for capabilities touching execution/injection/mutation.
- A `doctor` that is strictly read-only and a `setup` that is the only writer.

Avoid / adapt:
- REA's bring-your-own posture means it never automates installing Ghidra/IDA/JEB; a control plane that wants to "drive any desktop app" should still keep installers out of scope by default and treat the Hopper installer as the ceiling of what to automate (vendor-signed, user-approved, no license handling).
- The `hopper-demo-x11.py` fixed-coordinate click is fragile UI automation, acceptable only because it targets a vendor-offered demo button; do not generalize coordinate clicking for license dialogs.
- `always-bump-minor` with breaking changes allowed is unusual; for a control plane consumed by agents, prefer honest SemVer or explicit contract versioning.
- `install.sh` still trusts `api.github.com` and the npm registry over TLS with no signature on the tag->version mapping; acceptable for npm-provenance-backed packages but note it.

Boundary flags for the downstream project:
- Driving commercial RE tools (IDA, JEB, Hopper) through their APIs is permitted only within the user's license; JEB's EULA restricts sharing of decompiled output; ida-pro-mcp does not support IDA Free; Hopper personal vs computer licenses differ. A general control plane that scripts these tools must surface these terms and must not automate activation, key entry, or trial resets.
- REA explicitly places "automating acceptance of a vendor demo mode" on the allowed side and "license/DRM/anti-cheat bypass" on the excluded side; that is a useful precedent line.

## Constraints & risks

- `SECURITY.md:46` (SHA-1 corruption-check wording) contradicts the Linux PGP implementation; whichever is stale, documentation and code disagree.
- npx acquisition happens before REA code runs; REA "cannot validate package acquisition" (`docs/installation.md:1803`), hence the trusted-directory warning. Registrations created via npx depend on `_npx/<key>` cache survival (`docs/installation.md:485-525,1860-1863`).
- Hopper macOS integrity uses SHA-1 + length from the vendor feed (code signature is the real check); Linux uses PGP but the public key is fetched from the same vendor host and only the fingerprint is pinned in REA (acceptable, but key rotation at the vendor will break installs until REA updates).
- Shallow clone: could not date when PGP verification landed or confirm historical release workflow behavior; `docs/releasing.md` claims (OIDC, provenance) are partly INFER from absence of token secrets plus `id-token: write`.
- Plugin install and `rea setup` registrations can create two `rea` servers; docs tell users to pick one (`docs/installation.md:1722-1724`). `rea doctor` cannot see plugin-owned state.
- Codex fetches npm plugins without installing deps, so the tarball bundles production `node_modules` including PTY platform binaries (`docs/releasing.md:390-403`), enlarging the artifact; optional `frida` stays out.
- Many supported-client dialects (TOML, YAML, JSONC, Cordis patch dir for DeepSeek Harness) raise maintenance surface; each is a parser/serializer that must preserve comments.
- `always-bump-minor` means version numbers do not signal breaking changes.
- Linux Hopper install needs `pkexec` (polkit) and a private X display; unattended CI needs demo-dialog automation that depends on Hopper's dialog geometry (`UNEXPECTED_DISPLAY_GEOMETRY`, `UNSUPPORTED_DEMO_DIALOG` exit codes).

## Open questions

- Is npm publication actually using trusted publishing/OIDC with provenance attestations? Workflows show `id-token: write` and no token env; `publish-mcp` action uses `github-oidc`, but the npm step's provenance flag/attestation is not visible in the checkout (unknown).
- When was Linux PGP verification introduced relative to the SECURITY.md text? (git history unavailable in the shallow clone.)
- Does `verifyDetachedPgpSignature` check key expiry/revocation beyond what `openpgp.verify` does by default? Not inspected.
- How does `rea uninstall` interact with plugin-owned registrations? Docs say doctor cannot see them; uninstall behavior for plugins not read.
- Does Hopper's license agreement permit the X11 demo-button automation in CI? REA asserts compatibility (`docs/installation.md:1044-1046`) but the vendor text was not fetched here.

## Sources

- `install.sh`; `scripts/rea.mjs`; `scripts/plugin-mcp-launcher.mjs`; `scripts/prepack.mjs`; `scripts/generate-skill-metadata.mjs`; `scripts/generate-third-party-inventory.mjs`; `scripts/verify-release-checkpoint.mjs`; `scripts/lib/release-checkpoint.mjs`; `scripts/verify-repository-skill.mjs`; `scripts/verify-package-install.mjs`; `scripts/ci/prepare-npm-publication.mjs`; `scripts/hopper-demo-x11.py`
- `src/cli/setupCommands.ts`; `src/cli/interactiveSetup.ts`; `src/application/Setup.ts`, `SetupPlan.ts`, `SetupTypes.ts`, `SetupClients.ts`, `SetupClientConfiguration.ts`, `SetupRecoveryBackup.ts`, `SetupSkill.ts`, `SetupHost.ts`, `SupportedClients.ts`, `ClientRegistrationIdentity.ts`, `Update.ts`, `UpdateRuntime.ts`, `MacHopper.ts`, `LinuxHopper.ts`, `LinuxHopperPublisher.ts`, `HopperReleaseFeed.ts`, `HopperDownload.ts`
- `src/process/DetachedPgpSignature.ts`; `src/process/MacCodeSignature.ts`; `src/domain/hopperPublisher.ts`
- `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `.codex-plugin/plugin.json`, `.codex-plugin/mcp.json`, `.agents/plugins/marketplace.json`, `server.json`, `.agents/skills/reverse-engineer-anything/SKILL.md`
- `package.json`, `.npmrc`, `.nvmrc`, `.gitmodules`, `.husky/*`, `patches/`, `release-please-config.json`, `.release-please-manifest.json`, `.github/workflows/release.yml`, `.github/dependabot.yml`
- `README.md`, `SECURITY.md`, `DISCLAIMER.md`, `CONTRIBUTING.md`, `THIRD_PARTY_NOTICES.md`, `third_party/README.md`, `licenses/README.md`, `docs/releasing.md`, `docs/installation.md`, `docs/ida-provider.md`, `docs/kuna-provider.md`, `docs/jeb-analysis.md`
