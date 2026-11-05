//! `help-text@1` conventions (git parse-options, GNU getopt).
//!
//! A command whose usage block shows several forms of **the same** command (`usage: git branch …` /
//! `or: git branch (-d | -D) …`) switches authority by mode, so the unsplit native candidate is
//! `MIXED_AUTHORITY` until the overlay splits it. Forms of another command (`or: git show …` under `git log`) do
//! not count.

/// Counts the usage forms in `text` whose command words equal `native_path` (after the program name).
pub fn usage_forms(text: &str, native_path: &[String]) -> usize {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            l.strip_prefix("usage:")
                .or_else(|| l.strip_prefix("Usage:"))
                .or_else(|| l.strip_prefix("or:"))
        })
        .filter(|rest| {
            let words: Vec<&str> = rest.split_whitespace().skip(1).collect();
            !native_path.is_empty()
                && words.len() >= native_path.len()
                && words.iter().zip(native_path).all(|(w, n)| w == n)
        })
        .count()
}

/// Whether the usage block shows the command in more than one mode.
pub fn multi_mode(text: &str, native_path: &[String]) -> bool {
    usage_forms(text, native_path) >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_usage_forms() {
        let branch =
            "usage: git branch [<options>] [-r | -a]\n   or: git branch [<options>] [-r] (-d | -D) <branch-name>...\n";
        assert!(multi_mode(branch, &["branch".into()]));
        let log =
            "usage: git log [<options>] [<revision-range>] [[--] <path>...]\n   or: git show [<options>] <object>...\n";
        assert!(!multi_mode(log, &["log".into()]));
        let status = "usage: git status [<options>] [--] [<pathspec>...]\n";
        assert_eq!(usage_forms(status, &["status".into()]), 1);
    }
}
