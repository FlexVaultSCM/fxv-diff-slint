//! Parsing unified diff text into [`DiffSet`].
//!
//! The lexing is delegated to `diffy`, which handles the format's awkward corners: git
//! extended headers, binary markers, and the `\ No newline at end of file` convention. What
//! is added here is an owned model with per-line numbers resolved, which a viewer needs and
//! a patch applier does not.

// == External Crates
use diffy::patch_set::{FileOperation, ParseOptions, PatchKind, PatchSet, PatchSetParseError};
use snafu::{ResultExt, Snafu};

// == Internal Crates
use crate::model::{
    DiffLine, DiffSet, FileChange, FileContent, FileDiff, FileMode, Hunk, LineKind,
};

/// Everything that can go wrong reading a diff.
#[derive(Debug, Snafu)]
pub enum ParseError {
    /// The text is not a unified diff, or is one the parser cannot make sense of.
    ///
    /// The source is boxed rather than naming the underlying parser's error type, so that
    /// swapping or upgrading that parser is not a breaking change to this crate's API.
    #[snafu(display("malformed unified diff: {source}"))]
    Malformed {
        #[snafu(source(from(PatchSetParseError, Into::into)))]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Parses a unified diff, git-flavoured or plain.
///
/// Accepts a document covering any number of files.
pub fn parse_unified_diff(text: &str) -> Result<DiffSet, ParseError> {
    // A diff with no changes in it is a legitimate thing to be handed, not an error.
    if text.trim().is_empty() {
        return Ok(DiffSet::default());
    }

    // The two dialects each reject what the other accepts, so neither alone is enough:
    //
    //                        git dialect   plain dialect
    //   bare ---/+++ header   rejected      accepted
    //   rename                detected      reported as a modify
    //   mode change           detected      lost
    //   binary marker         detected      whole document rejected
    //
    // Git output is the common case and carries the most information, so it is tried first
    // and the plain dialect only picks up what it turns away.
    match collect(text, ParseOptions::gitdiff()) {
        Ok(files) if !files.is_empty() => Ok(DiffSet { files }),
        _ => collect(text, ParseOptions::unidiff()).map(|files| DiffSet { files }),
    }
}

fn collect(text: &str, opts: ParseOptions) -> Result<Vec<FileDiff>, ParseError> {
    PatchSet::parse(text, opts)
        .map(|patch| patch.context(MalformedSnafu).map(|p| convert_file(&p)))
        .collect()
}

fn convert_file(patch: &diffy::patch_set::FilePatch<'_, str>) -> FileDiff {
    let (left_path, right_path, change) = match patch.operation() {
        FileOperation::Create(p) => (None, Some(clean_path(p)), FileChange::Added),
        FileOperation::Delete(p) => (Some(clean_path(p)), None, FileChange::Removed),
        FileOperation::Modify { original, modified } => (
            Some(clean_path(original)),
            Some(clean_path(modified)),
            FileChange::Modified,
        ),
        FileOperation::Rename { from, to } => (
            Some(clean_path(from)),
            Some(clean_path(to)),
            FileChange::Renamed,
        ),
        FileOperation::Copy { from, to } => (
            Some(clean_path(from)),
            Some(clean_path(to)),
            FileChange::Copied,
        ),
    };

    let content = match patch.patch() {
        PatchKind::Binary(_) => FileContent::Binary,
        PatchKind::Text(text) => FileContent::Text {
            hunks: text.hunks().iter().map(convert_hunk).collect(),
        },
    };

    FileDiff {
        left_path,
        right_path,
        change,
        left_mode: patch.old_mode().map(convert_mode),
        right_mode: patch.new_mode().map(convert_mode),
        content,
    }
}

fn convert_mode(mode: &diffy::patch_set::FileMode) -> FileMode {
    use diffy::patch_set::FileMode as Src;
    match mode {
        Src::Regular => FileMode::Regular,
        Src::Executable => FileMode::Executable,
        Src::Symlink => FileMode::Symlink,
        Src::Gitlink => FileMode::Gitlink,
    }
}

fn convert_hunk(hunk: &diffy::Hunk<'_, str>) -> Hunk {
    let left_start = hunk.old_range().start() as u32;
    let right_start = hunk.new_range().start() as u32;

    // The header gives each side's starting line; per-line numbers come from walking the
    // hunk, since context advances both sides and a change advances only one.
    let mut left_no = left_start;
    let mut right_no = right_start;

    let lines = hunk
        .lines()
        .iter()
        .map(|line| {
            let (kind, raw) = match line {
                diffy::Line::Context(t) => (LineKind::Context, *t),
                diffy::Line::Delete(t) => (LineKind::Removed, *t),
                diffy::Line::Insert(t) => (LineKind::Added, *t),
            };

            // Content arrives with its line terminator still attached. A line missing one is
            // how the parser represents `\ No newline at end of file`: rather than flagging
            // it separately, it strips the newline from the line the marker followed.
            let (text, had_terminator) = strip_terminator(raw);

            let (left_line, right_line) = match kind {
                LineKind::Context => {
                    let pair = (Some(left_no), Some(right_no));
                    left_no += 1;
                    right_no += 1;
                    pair
                }
                LineKind::Removed => {
                    let pair = (Some(left_no), None);
                    left_no += 1;
                    pair
                }
                LineKind::Added => {
                    let pair = (None, Some(right_no));
                    right_no += 1;
                    pair
                }
            };

            DiffLine {
                kind,
                text: text.to_owned(),
                left_line,
                right_line,
                no_newline_at_eof: !had_terminator,
            }
        })
        .collect();

    Hunk {
        left_start,
        left_len: hunk.old_range().len() as u32,
        right_start,
        right_len: hunk.new_range().len() as u32,
        heading: hunk
            .function_context()
            .map(|c| strip_terminator(c).0.to_owned()),
        lines,
    }
}

/// Splits off a trailing line terminator, reporting whether there was one.
///
/// A carriage return is treated as part of the terminator rather than as content. It is a
/// line-ending artifact, and leaving it in place renders as a stray glyph or a phantom
/// column of whitespace.
fn strip_terminator(line: &str) -> (&str, bool) {
    match line.strip_suffix('\n') {
        Some(rest) => (rest.strip_suffix('\r').unwrap_or(rest), true),
        None => (line, false),
    }
}

/// Removes the `a/` and `b/` prefixes git puts on paths.
///
/// Only those exact prefixes are removed. Plain `diff -u` output carries real paths that must
/// be left alone, and stripping a leading component unconditionally would mangle them.
fn clean_path(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures are real `git diff` and `diff -u` output, generated from an actual
    /// repository. Hand-writing them invites headers whose line counts do not match their
    /// hunks, which the parser correctly rejects and which then looks like a parser bug.
    fn fixture(name: &str) -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
        std::fs::read_to_string(format!("{path}{name}.diff"))
            .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    fn one_file(name: &str) -> FileDiff {
        let set = parse_unified_diff(&fixture(name)).expect("parse");
        assert_eq!(set.files.len(), 1, "expected exactly one file in {name}");
        set.files.into_iter().next().unwrap()
    }

    #[test]
    fn strips_the_git_path_prefixes() {
        let file = one_file("git_modify");
        assert_eq!(file.left_path.as_deref(), Some("src/store.rs"));
        assert_eq!(file.right_path.as_deref(), Some("src/store.rs"));
        assert_eq!(file.change, FileChange::Modified);
    }

    #[test]
    fn keeps_the_hunk_heading() {
        let file = one_file("git_modify");
        assert_eq!(
            file.hunks()[0].heading.as_deref(),
            Some("use std::collections::HashMap;")
        );
    }

    #[test]
    fn records_hunk_ranges() {
        let file = one_file("git_modify");
        let hunk = &file.hunks()[0];
        assert_eq!((hunk.left_start, hunk.left_len), (2, 11));
        assert_eq!((hunk.right_start, hunk.right_len), (2, 11));
    }

    #[test]
    fn strips_the_leading_marker_from_line_text() {
        let file = one_file("git_modify");
        let lines = &file.hunks()[0].lines;
        let removed: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(removed[0], "        self.map.get(key)");
    }

    #[test]
    fn numbers_lines_by_walking_the_hunk() {
        let file = one_file("git_modify");
        let lines = &file.hunks()[0].lines;

        // Context advances both sides, a removal only the left, an addition only the right.
        for line in lines {
            match line.kind {
                LineKind::Context => {
                    assert!(line.left_line.is_some() && line.right_line.is_some());
                }
                LineKind::Removed => {
                    assert!(line.left_line.is_some() && line.right_line.is_none());
                }
                LineKind::Added => {
                    assert!(line.left_line.is_none() && line.right_line.is_some());
                }
            }
        }

        // The first line of the hunk carries the numbers from the header.
        assert_eq!(lines[0].left_line, Some(2));
        assert_eq!(lines[0].right_line, Some(2));

        // Each side's numbers are consecutive with no gaps or repeats.
        assert_consecutive(lines.iter().filter_map(|l| l.left_line), 2);
        assert_consecutive(lines.iter().filter_map(|l| l.right_line), 2);
    }

    fn assert_consecutive(numbers: impl Iterator<Item = u32>, first: u32) {
        for (n, expected) in numbers.zip(first..) {
            assert_eq!(n, expected, "line numbers must not skip or repeat");
        }
    }

    #[test]
    fn detects_a_rename() {
        let file = one_file("git_rename");
        assert_eq!(file.change, FileChange::Renamed);
        assert_eq!(file.left_path.as_deref(), Some("old_name.rs"));
        assert_eq!(file.right_path.as_deref(), Some("new_name.rs"));
        assert_eq!(file.display_path(), "new_name.rs");
    }

    #[test]
    fn detects_a_mode_change_with_no_content_change() {
        let file = one_file("git_mode");
        assert_eq!(file.left_mode, Some(FileMode::Regular));
        assert_eq!(file.right_mode, Some(FileMode::Executable));
        assert!(file.hunks().is_empty(), "a mode change has no hunks");
    }

    #[test]
    fn detects_a_binary_file() {
        let file = one_file("git_binary");
        assert!(file.is_binary());
        assert!(
            file.hunks().is_empty(),
            "binary files have nothing to render"
        );
    }

    #[test]
    fn detects_an_added_file() {
        let file = one_file("git_add");
        assert_eq!(file.change, FileChange::Added);
        assert_eq!(file.left_path, None);
        assert_eq!(file.right_path.as_deref(), Some("added.rs"));
    }

    #[test]
    fn detects_a_missing_trailing_newline() {
        let file = one_file("git_nonewline");
        let lines = &file.hunks()[0].lines;

        // Only the final line on each side lacks a terminator.
        let flagged: Vec<&str> = lines
            .iter()
            .filter(|l| l.no_newline_at_eof)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(flagged, vec!["gamma", "delta"]);
        assert!(lines
            .iter()
            .any(|l| l.text == "alpha" && !l.no_newline_at_eof));
    }

    #[test]
    fn treats_carriage_returns_as_part_of_the_terminator() {
        let file = one_file("git_crlf");
        for line in &file.hunks()[0].lines {
            assert!(!line.text.contains('\r'), "stray CR in {:?}", line.text);
        }
        let added: Vec<&str> = file.hunks()[0]
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, vec!["two modified"]);
    }

    #[test]
    fn accepts_plain_unified_diff_without_git_headers() {
        let set = parse_unified_diff(&fixture("plain_unified")).expect("parse");
        assert_eq!(set.files.len(), 1);
        assert!(!set.files[0].hunks().is_empty());
    }

    #[test]
    fn reads_a_multi_file_diff() {
        let set = parse_unified_diff(&fixture("git_full")).expect("parse");
        let paths: Vec<&str> = set.files.iter().map(|f| f.display_path()).collect();
        assert!(paths.contains(&"added.rs"), "got {paths:?}");
        assert!(paths.contains(&"new_name.rs"), "got {paths:?}");
        assert!(paths.contains(&"src/store.rs"), "got {paths:?}");
        assert!(
            set.files.len() >= 6,
            "got {} files: {paths:?}",
            set.files.len()
        );
    }

    #[test]
    fn an_empty_document_is_an_empty_diff_not_an_error() {
        assert!(parse_unified_diff("").expect("parse").is_empty());
        assert!(parse_unified_diff("   \n\n").expect("parse").is_empty());
    }

    #[test]
    fn rejects_text_that_is_not_a_diff() {
        assert!(parse_unified_diff("just some prose\nover two lines\n").is_err());
    }

    #[test]
    fn a_parse_failure_explains_itself_and_keeps_its_source() {
        let err = parse_unified_diff("just some prose\nover two lines\n").unwrap_err();

        let text = err.to_string();
        assert!(text.starts_with("malformed unified diff: "), "got {text:?}");
        assert!(
            text.len() > "malformed unified diff: ".len(),
            "no detail in {text:?}"
        );

        assert!(
            std::error::Error::source(&err).is_some(),
            "the underlying parser error should stay reachable"
        );
    }

    #[test]
    fn preserves_trailing_whitespace_in_content() {
        // Not from a fixture: git strips nothing, but an editor writing the fixture might.
        let text = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old  \n+new\t\n";
        let set = parse_unified_diff(text).expect("parse");
        let lines = &set.files[0].hunks()[0].lines;
        assert_eq!(lines[0].text, "old  ");
        assert_eq!(lines[1].text, "new\t");
    }
}
