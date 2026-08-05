//! The parsed representation of a diff.
//!
//! This is an owned, display-oriented model. It deliberately keeps the structure a unified
//! diff describes, rather than flattening it into rows: the row list depends on the layout
//! and on which gaps the user has expanded, and is built separately.

/// One or more file diffs, as found in a single unified diff document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSet {
    pub files: Vec<FileDiff>,
}

impl DiffSet {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Removed,
    Modified,
    Renamed,
    Copied,
}

/// A file's permission bits, for the mode changes git records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
    /// A submodule reference.
    Gitlink,
}

/// A file's contents, or the absence of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    Text {
        hunks: Vec<Hunk>,
    },
    /// Git reports binary files without content. There is nothing to render but the header.
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Path on the left, absent when the file was added.
    pub left_path: Option<String>,
    /// Path on the right, absent when the file was removed.
    pub right_path: Option<String>,
    pub change: FileChange,
    pub left_mode: Option<FileMode>,
    pub right_mode: Option<FileMode>,
    pub content: FileContent,
}

impl FileDiff {
    /// The path to show for this file: the right-hand one where there is one, since that is
    /// what the file is called after the change.
    pub fn display_path(&self) -> &str {
        self.right_path
            .as_deref()
            .or(self.left_path.as_deref())
            .unwrap_or("")
    }

    pub fn hunks(&self) -> &[Hunk] {
        match &self.content {
            FileContent::Text { hunks } => hunks,
            FileContent::Binary => &[],
        }
    }

    pub fn is_binary(&self) -> bool {
        matches!(self.content, FileContent::Binary)
    }
}

/// A contiguous run of changed lines plus the context around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// First line number covered on the left, 1-based.
    ///
    /// A hunk that only inserts covers no left lines, and by convention the header then names
    /// the line the insertion follows, so this can be the line *before* the hunk.
    pub left_start: u32,
    pub left_len: u32,
    pub right_start: u32,
    pub right_len: u32,
    /// The text trailing the `@@` markers. Git puts the enclosing function signature here.
    pub heading: Option<String>,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged, present on both sides.
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The line's content, without the leading marker and without its line terminator.
    pub text: String,
    /// Line number on the left, 1-based. Absent for added lines.
    pub left_line: Option<u32>,
    /// Line number on the right, 1-based. Absent for removed lines.
    pub right_line: Option<u32>,
    /// The file this line came from does not end with a newline, and this is its last line.
    /// Diffs render this as a `\ No newline at end of file` marker.
    pub no_newline_at_eof: bool,
}
