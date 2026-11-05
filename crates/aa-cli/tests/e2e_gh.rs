//! Real `gh` through the cobra-complete harvester. Runs when `gh` is on PATH; otherwise prints a skip reason.
//! The overlay points `GH_CONFIG_DIR` into the private root (a relative path resolves against the private cwd),
//! so the user's gh config and tokens are never read, and confinement denies the network. `GH_TELEMETRY=0`: without
//! it gh writes `~/.local/state/gh/device-id` on every run, which L3 rightly reports as SIDE_EFFECT.

mod common;

use common::{Engine, admission, skip, which};

const GH_OVERLAY: &str = r#"schema: aa.overlay/v1
target: gh
display_name: GitHub CLI
mcp_prefix: gh
authorization: "e2e test: read-only gh alias listing with a private, empty config dir"
harvest:
  - harvester: cobra-complete@1
    commands: [alias]
defaults:
  binding:
    env: { GH_CONFIG_DIR: ".gh-config", GH_TELEMETRY: "0", GH_NO_UPDATE_NOTIFIER: "1", GH_PROMPT_DISABLED: "1", NO_COLOR: "1" }
    deadline_ms: 15000
    usage_exit_code: 1
    usage_stderr_marker: "unknown flag"
capabilities:
  gh.alias.list:
    examples:
      - { input: {} }
      - { input: {} }
"#;

#[test]
#[ignore = "needs integration"]
fn gh_alias_list_servable_under_confinement() {
    if which("gh").is_none() {
        return skip("gh_alias_list_servable_under_confinement", "gh is not on PATH");
    }
    let e = Engine::start();
    let overlay = e.root.join("gh.aa.yaml");
    std::fs::write(&overlay, GH_OVERLAY).unwrap();
    e.ok(&[
        "target",
        "add",
        "gh",
        "--exec",
        "gh",
        "--harvester",
        "cobra-complete",
        "--overlay",
        overlay.to_str().unwrap(),
        "--authorization",
        "e2e test",
    ]);
    e.ok(&["discover", "gh"]);
    let caps = e.capabilities("gh");
    assert!(caps.contains_key("gh.alias.list"), "{:?}", caps.keys());

    e.ok(&["verify", "gh", "--only", "gh.alias.list"]);
    let caps = e.capabilities("gh");
    let list = &caps["gh.alias.list"];
    assert_eq!(admission(list), "ADMISSION_STATE_SERVABLE", "{list}\n{}", e.log_text());
    let status = e.ok(&["status"]);
    if status["confinementAvailable"] == true {
        let desc = e.ok(&["describe", "gh.alias.list"]);
        assert_eq!(
            desc["capability"]["admission"]["requiredRuntimeIsolation"], "ISOLATION_CONFINED",
            "{desc}"
        );
    } else {
        skip(
            "gh_alias_list_servable_under_confinement",
            "Seatbelt unavailable: confinement half not checked",
        );
    }
}
