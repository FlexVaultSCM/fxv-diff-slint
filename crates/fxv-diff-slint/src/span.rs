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

/// Which file a line is numbered in.
///
/// **Serialise these as 0 for `Left` and 1 for `Right`.** A host storing spans should write
/// those numbers rather than the names, and the reason is that this type is provisional. What
/// a span really needs is to say which document its line number indexes into, and a diff
/// happens to have two. The same machinery drives a viewer showing one document, where
/// "left" and "right" are meaningless, so this becomes an identifier rather than a pair. Values
/// written as 0 and 1 survive that; values written as "left" and "right" have to be migrated.
///
/// The wider generalisation is not just this type. A row carries a line number per side, a
/// hidden count, a gap state, and a reference into a hunk, so a viewer with one document would
/// leave most of a row inert. That is a change to make deliberately and all at once, not by
/// renaming this in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
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
    pub side: Side,
    /// 1-based, matching the numbers the diff itself carries.
    pub line: u32,
    pub extent: SourceCharExtent,
}
