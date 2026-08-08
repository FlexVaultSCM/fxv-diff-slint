//! The parsed representation of a diff.
//!
//! This is an owned, display-oriented model. It deliberately keeps the structure a unified
//! diff describes, rather than flattening it into rows: the row list depends on the layout
//! and on which gaps the user has expanded, and is built separately.

// == Internal Crates
use crate::text::LineEnding;

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
    /// Runs of lines a host is fetching, or has failed to fetch. Lines that arrived are not
    /// here: they go into the hunks, because by then they are lines like any other.
    pub fetches: Vec<Fetch>,
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

    /// The line a row came from, for reading its original text.
    pub fn line(&self, at: LineRef) -> Option<&DiffLine> {
        self.hunks()
            .get(at.hunk as usize)?
            .lines
            .get(at.line as usize)
    }

    /// Adds context that was hidden, opening up part of a gap.
    ///
    /// The lines go into the hunks rather than beside them, because a gap is only ever the
    /// distance between one hunk and the next. Growing a hunk closes that distance, so the gap
    /// shrinks, splits or disappears on its own with nothing keeping track of it. This is what
    /// widening a diff's context does, and hunks that meet are merged for the same reason.
    ///
    /// Lines already covered by a hunk are ignored, so asking twice is harmless.
    ///
    /// Delivering lines ends any fetch outstanding for them, whether or not they turned out to
    /// be needed.
    pub fn expand(
        &mut self,
        left_start: u32,
        right_start: u32,
        lines: impl IntoIterator<Item = String>,
    ) {
        let added: Vec<DiffLine> = lines
            .into_iter()
            .enumerate()
            .map(|(offset, text)| {
                let offset = offset as u32;
                DiffLine {
                    kind: LineKind::Context,
                    text,
                    left_line: Some(left_start + offset),
                    right_line: Some(right_start + offset),
                    line_ending: LineEnding::Lf,
                    origin: LineOrigin::Expanded,
                }
            })
            .collect();

        if added.is_empty() {
            return;
        }
        let count = added.len() as u32;
        let end = left_start + count;

        // These lines are here now, so nothing is outstanding for them. A fetch that merely
        // overlaps goes too: the host delivered a different run from the one it recorded, and
        // keeping part of the old record would claim work is still in flight that nobody has
        // said is.
        self.fetches.retain(|f| {
            f.right_start >= right_start + count || f.right_start + f.count <= right_start
        });

        let FileContent::Text { hunks } = &mut self.content else {
            return;
        };

        // Early out if we don't need these lines
        if covered(hunks, left_start, count) {
            return;
        }

        if let Some(before) = hunks
            .iter_mut()
            .find(|h| h.left_start + h.left_len == left_start)
        {
            before.lines.extend(added);
            before.left_len += count;
            before.right_len += count;
        } else if let Some(after) = hunks.iter_mut().find(|h| h.left_start == end) {
            after.lines.splice(0..0, added);
            after.left_start = left_start;
            after.right_start = right_start;
            after.left_len += count;
            after.right_len += count;
        } else {
            // Neither end of the run touches a hunk, so it becomes one: a hunk of pure
            // context, which splits the gap it came out of in two.
            let at = hunks
                .iter()
                .position(|h| h.left_start > left_start)
                .unwrap_or(hunks.len());
            hunks.insert(
                at,
                Hunk {
                    left_start,
                    left_len: count,
                    right_start,
                    right_len: count,
                    heading: None,
                    lines: added,
                },
            );
        }

        merge_touching(hunks);
    }

    /// Records that somebody has gone to fetch a run of hidden lines, so the gap can say so.
    ///
    /// Replaces any earlier record for the same run, which is what makes retrying a failed
    /// fetch the same call as starting one.
    pub fn fetch_started(&mut self, right_start: u32, count: u32) {
        self.set_fetch(right_start, count, FetchState::Waiting);
    }

    /// Records that a fetch did not work. `why` is shown on the gap row.
    pub fn fetch_failed(&mut self, right_start: u32, count: u32, why: impl Into<String>) {
        self.set_fetch(right_start, count, FetchState::Failed(why.into()));
    }

    /// Forgets every failed fetch.
    ///
    /// A new attempt supersedes the reasons old ones gave, and leaving them behind means a gap
    /// reporting a failure that nobody is still waiting on. Whether to call this is the host's
    /// choice: nothing here decides when a failure has stopped being true.
    pub fn clear_failed_fetches(&mut self) {
        self.fetches
            .retain(|f| !matches!(f.state, FetchState::Failed(_)));
    }

    /// Forgets a fetch without filling it in, returning the gap to plain hidden lines.
    pub fn fetch_abandoned(&mut self, right_start: u32, count: u32) {
        self.fetches
            .retain(|f| f.right_start != right_start || f.count != count);
    }

    fn set_fetch(&mut self, right_start: u32, count: u32, state: FetchState) {
        match self
            .fetches
            .iter_mut()
            .find(|f| f.right_start == right_start && f.count == count)
        {
            Some(existing) => existing.state = state,
            None => self.fetches.push(Fetch {
                right_start,
                count,
                state,
            }),
        }
    }
}

/// Whether a run of lines is already part of some hunk.
fn covered(hunks: &[Hunk], left_start: u32, count: u32) -> bool {
    hunks
        .iter()
        .any(|h| left_start >= h.left_start && left_start + count <= h.left_start + h.left_len)
}

/// Joins hunks that have grown until they meet.
fn merge_touching(hunks: &mut Vec<Hunk>) {
    let mut i = 0;
    while i + 1 < hunks.len() {
        let joins = hunks[i].left_start + hunks[i].left_len == hunks[i + 1].left_start;
        if joins {
            let next = hunks.remove(i + 1);
            hunks[i].left_len += next.left_len;
            hunks[i].right_len += next.right_len;
            hunks[i].lines.extend(next.lines);
            // The surviving hunk keeps its own heading; the one absorbed described a place
            // that is no longer the start of anything.
        } else {
            i += 1;
        }
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

/// A run of hidden lines somebody is trying to open up.
///
/// Only ever describes lines that are not there yet. Once they arrive they become part of a
/// hunk and the fetch is dropped, so this never duplicates content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    /// First line of the run, numbered on the right.
    pub right_start: u32,
    pub count: u32,
    pub state: FetchState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchState {
    Waiting,
    /// Gave up, with something to show for why.
    Failed(String),
}

/// Points at one line of a parsed diff.
///
/// A row keeps one of these rather than a second copy of the text, so that copying can resolve
/// back to what the file actually contains. The row's own text has been rendered for display:
/// tabs expanded, and whitespace possibly drawn as something other than itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRef {
    pub hunk: u32,
    /// Index within the hunk's lines, not a line number in the file.
    pub line: u32,
}

/// Where a line came from.
///
/// Everything a diff contained is `Diff`. Context fetched later to open up a gap is `Expanded`,
/// so nothing is lost by keeping both in the same hunks: what the diff actually said is still
/// exactly recoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Diff,
    Expanded,
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
    /// How the line ended, with the terminator itself already stripped from `text`.
    pub line_ending: LineEnding,
    pub origin: LineOrigin,
}

impl DiffLine {
    /// Whether the file this line came from ends without a newline, this being its last line.
    pub fn no_newline_at_eof(&self) -> bool {
        self.line_ending == LineEnding::None
    }
}
