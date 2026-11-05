//! Hard exclusions (`exclusions@1`, architecture §5.7): evaluated before policy; config can add, never remove.
//!
//! Matching is by whole lowercase words (`[a-z0-9]+`) of the target id/name, the capability id and the given
//! text, so `serialize` or `registry` never match `serial` / `register`.

use crate::pb;

/// Rule-table id recorded on exclusion findings.
pub const RULE: &str = "rule:exclusions@1";

/// `(category, single words, adjacent word pairs)`.
type Row = (
    &'static str,
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);
const TABLE: &[Row] = &[
    (
        "licensing",
        &[
            "license",
            "licence",
            "licenses",
            "licences",
            "licensing",
            "relicense",
            "unlicense",
        ],
        &[],
    ),
    (
        "activation",
        &["activate", "activation", "activations", "deactivate", "reactivate"],
        &[],
    ),
    (
        "registration",
        &["register", "registration", "unregister", "reregister"],
        &[],
    ),
    ("drm", &["drm", "fairplay", "widevine", "playready"], &[]),
    (
        "serial",
        &["serial", "serials", "keygen", "productkey", "serialkey"],
        &[("product", "key"), ("serial", "key"), ("license", "key")],
    ),
    ("anti-cheat", &["anticheat"], &[("anti", "cheat")]),
    (
        "security-product",
        &["edr", "antivirus", "antimalware"],
        &[("anti", "virus"), ("anti", "malware")],
    ),
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// The exclusion category of a list of words, if any.
fn category(words: &[String]) -> Option<&'static str> {
    for (cat, singles, pairs) in TABLE {
        if words.iter().any(|w| singles.contains(&w.as_str())) {
            return Some(cat);
        }
        if words
            .windows(2)
            .any(|p| pairs.iter().any(|(a, b)| p[0] == *a && p[1] == *b))
        {
            return Some(cat);
        }
    }
    None
}

/// Returns the exclusion category name if `capability_id`/`text` hits a hard exclusion.
pub fn excluded(target: &pb::Target, capability_id: &str, text: &str) -> Option<&'static str> {
    [target.id.as_str(), target.display_name.as_str(), capability_id, text]
        .iter()
        .find_map(|s| category(&words(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str) -> pb::Target {
        pb::Target {
            id: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn keywords_by_word() {
        assert_eq!(excluded(&t("app"), "app.license.activate", ""), Some("licensing"));
        assert_eq!(
            excluded(&t("app"), "app.account", "Activate this copy"),
            Some("activation")
        );
        assert_eq!(excluded(&t("app"), "app.x", "enter your product key"), Some("serial"));
        assert_eq!(excluded(&t("app"), "app.drm.strip", ""), Some("drm"));
        assert_eq!(excluded(&t("game"), "game.anti-cheat.off", ""), Some("anti-cheat"));
        assert_eq!(excluded(&t("git"), "git.status", "Show the working tree status"), None);
        assert_eq!(excluded(&t("x"), "x.serialize", "registry of things"), None);
    }
}
