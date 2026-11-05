//! `aa-harvest`: discovery harvesters (architecture §8, §11, build-plan §3.3).
//!
//! - `cli-anything-registry@1` (STATIC) and [`cli_anything::import_harness`]: registry + harness checkout;
//! - `click-info@1` (INTROSPECT): the `aa-click-info/1` Python shim over `to_info_dict()`;
//! - `cobra-complete@1` (INTROSPECT): recursive `__complete` crawl;
//! - `help-text@1` (INTROSPECT): git parse-options and GNU `-h` text.
//!
//! `harvest()` is effectful and runs only through the supplied `ProcessRunner`; `candidates()` is pure over
//! the recorded bytes, so CI replays fixtures with no Python, git or gh.

pub mod cli_anything;
mod click_info;
mod cobra;
mod common;
pub mod help_text;
pub mod schema;

pub use cli_anything::CliAnythingRegistryHarvester;
pub use click_info::ClickInfoHarvester;
pub use cobra::CobraCompleteHarvester;
pub use help_text::HelpTextHarvester;

/// The aa-click-info/1 shim, written into a private root per run.
pub const CLICK_INFO_SHIM: &str = include_str!("../shims/aa_click_info.py");

/// The four built-in harvesters.
pub fn builtin_harvesters() -> Vec<std::sync::Arc<dyn aa_core::api::Harvester>> {
    vec![
        std::sync::Arc::new(CliAnythingRegistryHarvester),
        std::sync::Arc::new(ClickInfoHarvester),
        std::sync::Arc::new(CobraCompleteHarvester),
        std::sync::Arc::new(HelpTextHarvester),
    ]
}
