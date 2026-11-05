//! The real CLI-Anything LibreOffice harness. Opt-in:
//!   AA_TEST_CLI_ANYTHING=/Users/nhanclie/CLI-Anything AA_TEST_LO_PYTHON=<venv>/bin/python \
//!     cargo test -p aa-cli --test e2e_libreoffice
//! The operator pre-installs the harness venv; the engine never installs anything and never runs `install_cmd`.

mod common;

use common::{Engine, admission, skip};

fn env_path(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[test]
#[ignore = "needs integration"]
fn libreoffice_harness_end_to_end() {
    let (Some(repo), Some(python)) = (env_path("AA_TEST_CLI_ANYTHING"), env_path("AA_TEST_LO_PYTHON")) else {
        return skip(
            "libreoffice_harness_end_to_end",
            "set AA_TEST_CLI_ANYTHING (CLI-Anything checkout) and AA_TEST_LO_PYTHON (venv python with the harness)",
        );
    };
    let e = Engine::start();
    let imported = e.ok(&[
        "import",
        "cli-anything",
        &repo,
        "--harness",
        "libreoffice",
        "--python",
        &python,
        "--authorization",
        "owner workstation; MPL-2.0 app; Apache-2.0 harness",
    ]);
    let h = imported["harnesses"]
        .as_array()
        .and_then(|v| v.iter().find(|h| h["name"] == "libreoffice").cloned())
        .unwrap_or_else(|| panic!("no libreoffice harness imported: {imported}"));
    assert_eq!(h["targetId"], "cli-anything.libreoffice", "{h}");
    assert!(
        h["installCmdArtifactSha256"].as_str().is_some_and(|s| !s.is_empty()),
        "install_cmd not stored: {h}"
    );

    let d = e.ok(&["discover", "cli-anything.libreoffice"]);
    assert_eq!(d["counts"]["candidates"], 40, "{}", d["counts"]);

    e.ok(&["verify", "cli-anything.libreoffice"]);
    let caps = e.capabilities("cli-anything.libreoffice");
    for id in [
        "cli-anything.libreoffice.writer.list",
        "cli-anything.libreoffice.document.info",
    ] {
        let c = caps
            .get(id)
            .unwrap_or_else(|| panic!("{id} missing: {:?}", caps.keys()));
        assert_eq!(admission(c), "ADMISSION_STATE_SERVABLE", "{id}: {c}");
    }

    // Nothing that ran (intents and action records) ever involved pip.
    for entry in e.audit_all() {
        for body in ["intent", "action"] {
            if let Some(b) = entry.get(body) {
                let text = b.to_string();
                assert!(
                    !text.contains("pip install") && !text.contains("\"pip\""),
                    "pip in a dispatched action: {b}"
                );
                let cap = b["capabilityId"].as_str().unwrap_or_default();
                assert!(!cap.contains("install"), "an install action ran: {b}");
            }
        }
    }
}
