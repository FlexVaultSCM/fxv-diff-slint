//! Finding text in what a pane is showing.
//!
//! Nothing here is part of the widget. It exists to drive the highlight channels from outside,
//! which is what a host does: find the positions, hand them over, let the view draw them.
//!
//! Matches are found in source text and converted to display columns, never found in the text
//! a row draws. A rendered line has its tabs expanded and, with whitespace display on, its
//! spaces replaced by proxy glyphs, so searching it would have a query for a space match a
//! middle dot and a query containing a tab match nothing at all.

// == Std crates
use std::ops::Range;

// == Internal Crates
use fxv_diff_slint::{
    Channel, DisplayColumnExtent, Document, FileDiff, RenderOptions, RowModel, map_span,
};

// == Crate
use crate::panes::{Tab, Which};

/// The channel this application paints search matches in.
///
/// Numbered from the first the library leaves free rather than picked, so that the library
/// taking another channel later cannot quietly land on top of this one.
pub const SEARCH: Channel = Channel(Channel::FIRST_FREE.0);

/// The one match being stepped through, painted over the rest.
///
/// A higher channel than `SEARCH`, so it draws on top. The two are set separately: the whole
/// set changes only when the query does, while stepping touches a row or two.
pub const CURRENT: Channel = Channel(Channel::FIRST_FREE.0 + 1);

/// One match, kept so the find controls can step through them in the order they are read.
pub struct Found {
    pub which: Which,
    pub row: usize,
    pub extent: DisplayColumnExtent,
}

/// Every match of the current query, and which one is current.
///
/// Each tab is searched and stepped on its own, so every operation names one. Nothing outside
/// reaches the fields: which of the two pairs a tab means is this type's business, and having
/// callers branch on it was how the same `if` ended up written in five places.
#[derive(Default)]
pub struct Find {
    diff: Vec<Found>,
    plain: Vec<Found>,
    at_diff: usize,
    at_plain: usize,
}

impl Find {
    /// Replaces everything found in one tab, starting again from its first match.
    pub fn replace(&mut self, tab: Tab, found: Vec<Found>) {
        match tab {
            Tab::Standalone => {
                self.plain = found;
                self.at_plain = 0;
            }
            Tab::Diff => {
                self.diff = found;
                self.at_diff = 0;
            }
        }
    }

    pub fn matches(&self, tab: Tab) -> &[Found] {
        match tab {
            Tab::Standalone => &self.plain,
            Tab::Diff => &self.diff,
        }
    }

    /// Which match is current, counted from zero.
    pub fn at(&self, tab: Tab) -> usize {
        match tab {
            Tab::Standalone => self.at_plain,
            Tab::Diff => self.at_diff,
        }
    }

    pub fn current(&self, tab: Tab) -> Option<&Found> {
        self.matches(tab).get(self.at(tab))
    }

    /// Steps to another match, wrapping at both ends.
    ///
    /// `step` is 1 for the next and -1 for the previous. A tab with no matches does not move,
    /// since there is nothing to move to and the remainder would divide by zero.
    pub fn advance(&mut self, tab: Tab, step: isize) {
        let count = self.matches(tab).len();
        if count == 0 {
            return;
        }
        // Wraps at both ends. With 51 matches: from 50 forwards, 51 comes back to 0; from 0
        // backwards, -1 comes back to 50.
        //
        // Signed arithmetic, because that -1 cannot be reached in a `usize` at all: 0 - 1
        // panics in debug and wraps to a colossal number in release.
        //
        // `rem_euclid` and not `%`, because Rust's `%` is a remainder that keeps the sign of
        // the left side, so -1 % 51 is -1 rather than 50. Cast back, that is an index no row
        // has, and stepping back from the first match would quietly select nothing instead of
        // the last. `rem_euclid` always lands in 0..count, so the cast is safe to index with.
        let next = (self.at(tab) as isize + step).rem_euclid(count as isize) as usize;
        match tab {
            Tab::Standalone => self.at_plain = next,
            Tab::Diff => self.at_diff = next,
        }
    }
}

/// Where a query occurs in the lines a diff pane is showing.
pub fn diff_matches(
    model: &RowModel,
    file: &FileDiff,
    opts: &RenderOptions,
    query: &str,
    pane: Which,
) -> Vec<(usize, DisplayColumnExtent)> {
    let mut found = Vec::new();
    for (row, displayed) in model.rows().iter().enumerate() {
        // A gap, a filler or a header stands for no line, so there is nothing to search.
        let Some(source) = displayed.source else {
            continue;
        };
        let Some(line) = file.line(source) else {
            continue;
        };

        for chars in match_ranges(&line.text, query) {
            // Source characters are not display columns: a tab is one character and several
            // columns, and showing whitespace changes the count again. The conversion is the
            // same one a stored selection goes through.
            let columns = map_span(&line.text, chars.clone(), opts);
            if let Some((document, number)) = displayed.id {
                log_match(pane, row, document, number, &chars, &columns);
            }
            found.push((
                row,
                DisplayColumnExtent::Columns(columns.start as u32..columns.end as u32),
            ));
        }
    }
    found
}

/// Character ranges where `query` occurs in `text`, counted in characters rather than bytes.
///
/// Characters, because that is what a span is measured in. Case sensitive and literal: a
/// viewer for testing highlights wants a query that means exactly what it says.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let width = query.chars().count();

    // Counted forward from the previous match rather than from the start of the line, so a
    // line with many matches costs one walk rather than one per match.
    let mut out = Vec::new();
    let mut chars = 0;
    let mut counted_to = 0;
    for (byte, _) in text.match_indices(query) {
        chars += text[counted_to..byte].chars().count();
        counted_to = byte;
        out.push(chars..chars + width);
    }
    out
}

/// Reports one match on stderr, in both the durable form and the drawn one.
///
/// The durable half is what a host would store: which document, a line number, and a character
/// range, none of which move when a gap opens or the whitespace options change. The drawn half
/// is the row and the columns it landed on, which do.
///
/// The document is printed as its number rather than a name, because the number is what a host
/// stores and what it means is the row provider's business, not this logger's.
pub fn log_match(
    pane: Which,
    row: usize,
    document: Document,
    line: u32,
    chars: &Range<usize>,
    columns: &Range<usize>,
) {
    eprintln!(
        "find: pane={pane:?} span=doc{}:{line} chars={}..{} drawn at row={row} columns={}..{}",
        document.0, chars.start, chars.end, columns.start, columns.end
    );
}
