//! Small shared helpers used across the converters and both front-ends.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use std::path::Path;

/// Returns `true` when `output` and `input` designate the same file on disk,
/// so that writing `output` would clobber the source `input`.
///
/// When both paths resolve on disk (the reliable case, e.g. the output already
/// exists) the OS canonical paths are compared. Otherwise a normalized lexical
/// comparison is used so a not-yet-created output that still names the input is
/// rejected as well.
pub fn paths_refer_to_same_file(input: &str, output: &str) -> bool {
    let (ip, op) = (Path::new(input), Path::new(output));
    if let (Ok(a), Ok(b)) = (ip.canonicalize(), op.canonicalize()) {
        return a == b;
    }
    normalize_lexical(input) == normalize_lexical(output)
}

/// Normalize a path string for a best-effort comparison when it cannot be
/// canonicalized: unify separators, drop a trailing slash, and (on Windows,
/// where the file system is case-insensitive) lower-case it.
fn normalize_lexical(p: &str) -> String {
    let unified = p.replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    #[cfg(windows)]
    {
        trimmed.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_names_are_not_the_same_file() {
        assert!(!paths_refer_to_same_file("game.vsf", "game_vs.prg"));
    }

    #[test]
    fn identical_nonexistent_paths_match_lexically() {
        assert!(paths_refer_to_same_file("nope/game.prg", "nope/game.prg"));
    }

    #[test]
    fn separator_and_case_differences_match_on_windows() {
        // On Windows these name the same (nonexistent) file; on Unix they differ.
        let same = paths_refer_to_same_file("dir/Game.PRG", "dir\\game.prg");
        assert_eq!(same, cfg!(windows));
    }
}
