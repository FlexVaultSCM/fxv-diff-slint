//! Naming part of a file, in terms that outlive the view.
//!
//! Everything here is durable. A value of these types survives a gap opening, a rebuild, a
//! change of layout, and a restart, because nothing identifying it comes from what is on
//! screen. That is what makes them safe for a host to store against a review comment and hand
//! back weeks later.
//!
//! Three things that look like identity are not, and none of them appear here:
//!
//! - Row indices move as soon as a gap opens.
//! - Hunk indices and offsets within them move with them, because opened lines are inserted
//!   into hunks rather than kept beside them.
//! - Display columns move when the tab width or the whitespace options change, since both
//!   alter how many columns a character occupies.
//!
//! Line numbers and source character offsets survive all of that.

// == Std
use std::ops::Range;

/// Which document a line number indexes into.
///
/// A number rather than a name, because how many documents a pane is showing and what they mean
/// is the business of whatever supplied its rows. A diff has two, a plain listing has one, and
/// something showing three revisions side by side would have three.
///
/// **This number is the stored form.** A host writing a span down writes the number, so the
/// meaning has to be fixed by whatever produces the rows and stay fixed. See `Document::BEFORE`
/// and `Document::AFTER`, which are what a diff calls its two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Document(pub u32);

impl Document {
    /// The only document, for a pane showing one.
    pub const ONLY: Document = Document(0);
}

/// How much of one line a span covers, in that line's source characters.
///
/// The two variants differ by whether the line's ending is part of the span, which is the
/// distinction that matters twice over: a span holding the ending is drawn out to the edge of
/// the pane, because the newline is what the rest of that row stands for, and it is the span
/// that emits a line break when the selection is copied. Splitting them here rather than
/// carrying a range and a flag means "stops mid-line but claims the newline" cannot be said.
///
/// Counted in characters, not display columns: a tab is one character here however many
/// columns it is drawn across.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCharExtent {
    /// Characters `range` of the line, stopping inside it. No line ending.
    Columns(Range<u32>),
    /// From `from` to the end of the line, the line ending included. `from: 0` is a whole line.
    ToEnd { from: u32 },
}

/// A run of characters on one line, named the way a file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSpan {
    /// Which document `line` is numbered in.
    pub document: Document,
    /// 1-based, matching the numbers the diff itself carries.
    pub line: u32,
    pub extent: SourceCharExtent,
}
