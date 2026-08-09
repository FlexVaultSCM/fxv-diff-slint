//! A row as a pane will draw it, and the ranges painted over it.
//!
//! One type describes a position in a pane; the others describe what is drawn behind its text.
//! They live together because a highlight is meaningless except against a row, and separating
//! them by topic made the two halves of the view depend on each other in a circle.
//!
//! Everything here is transient. A row index and a display column both move when a gap opens
//! or the whitespace options change, so none of it survives the frame that produced it.
//! Storing one is a mistake; store a `LineSpan`.

// == Std crates
use std::ops::Range;

// == External Crates
use slint::SharedString;

// == Internal Crates
use crate::diff::layout::GapState;
use crate::diff::model::LineRef;
use crate::span::Document;
use crate::text::LineEnding;
use crate::text::{render_line, RenderOptions};

/// How many numbers a gutter can show at once.
///
/// Two, because that is what an inline diff needs and nothing here wants more. The cap lives
/// at this level on purpose: a layout has no opinion about gutters, and the widget struct
/// carries two plain integers rather than a list, which would cost every row an allocation to
/// hold what fits in eight bytes.
pub const GUTTER_COLUMNS: usize = 2;

/// What a row is, which is what picks how it is drawn.
///
/// An open set, like [`Channel`]. The view holds a background per class and the numbers mean
/// whatever the thing producing the rows decides, so a pane can show classes of row this crate
/// has never heard of. The six below are the ones a diff produces.
///
/// A class says how a row looks, not how it behaves. What the pane needs to know in order to
/// lay a row out is stated on the row itself, in `numbered` and `full_width`, so that a class
/// the pane has never seen still lays out correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowClass(pub u32);

impl RowClass {
    /// Unchanged content, shown to give a change context.
    pub const CONTEXT: RowClass = RowClass(0);
    pub const ADDED: RowClass = RowClass(1);
    pub const REMOVED: RowClass = RowClass(2);
    /// Content that exists but is not shown.
    pub const GAP: RowClass = RowClass(3);
    /// Nothing on this side. Keeps the two panes of a split view in step.
    pub const FILLER: RowClass = RowClass(4);
    /// Names the file rather than anything in it.
    pub const HEADER: RowClass = RowClass(5);

    /// The first class this crate does not use itself.
    ///
    /// A host numbers its own from here, so that this crate taking another class later cannot
    /// collide with one already in use.
    pub const FIRST_FREE: RowClass = RowClass(6);
}

/// A run of lines that exists but is not shown, and what is happening to it.
///
/// Only a diff produces these. It lives beside the row rather than inside it so that a row
/// which is a line stays a line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gap {
    /// How many lines are hidden.
    pub hidden: u32,
    pub state: GapState,
    /// The hunk heading this gap precedes, or why fetching it failed.
    pub note: SharedString,
    /// Where the hidden run starts, on each side. What the controls ask for.
    pub start: (u32, u32),
    /// The run being fetched, numbered on the right, when one is.
    pub pending: Option<(u32, u32)>,
}

/// One row as a pane will draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayedRow {
    pub class: RowClass,
    /// Whether the gutter shows this row's numbers.
    ///
    /// A filler keeps its gutter but shows no number: it stands opposite a real line in the
    /// other pane, and losing the gutter would put the two panes out of step, but it names no
    /// line of its own.
    pub numbered: bool,
    /// Whether this row's content runs across the gutter as well.
    ///
    /// For a row that is about the document rather than a line in it.
    pub full_width: bool,
    /// Gutter numbers, in the order the view draws them. A pane showing one file fills only
    /// the first.
    pub numbers: [Option<u32>; GUTTER_COLUMNS],
    /// Which file names this row and the number it has there, for describing a selection over
    /// it. Absent on rows that stand for no line.
    ///
    /// Settled here rather than asked for later, because a pane knows which file it is showing
    /// and an entry knows which side a line came from.
    pub id: Option<(Document, u32)>,
    /// The line this row was rendered from, for anything needing the original text.
    pub source: Option<LineRef>,
    /// Display text, tabs already expanded. Empty for fillers and gaps.
    pub text: SharedString,
    /// Columns the text occupies.
    pub columns: u32,
    /// What this row stands for, when it stands for content rather than showing it.
    ///
    /// `None` on an ordinary line, which is every row a pane showing a plain file has. Grouped
    /// rather than spread across the row so that a pane with no gaps carries one empty option
    /// instead of five fields it will never fill.
    pub gap: Option<Gap>,
    /// Whether a selection may cover this row. Gaps and headers break a selection; a filler
    /// does not, being a real position in a pane that simply holds no line.
    pub selectable: bool,
    /// The run of selectable rows this one belongs to, as row indices.
    ///
    /// Not the selected run: the run a selection is *allowed* to cover. A drag may not cross a
    /// gap or a header, so it is confined to the run it began in. Empty on a row that is not
    /// selectable at all.
    ///
    /// Carried per row because the row the pointer went down on is the one that has to clamp
    /// the other end, and a row knows only itself. Filled in by `RowModel`, which is the first
    /// thing to see the rows in order.
    pub selectable_run: Range<u32>,
}

impl DisplayedRow {
    /// One line of ordinary content, numbered and selectable.
    ///
    /// What a view showing a plain file builds its rows from, rather than going through a
    /// layout that exists to describe a change.
    ///
    /// `ending` is how the line was terminated in the file, which the text alone cannot say
    /// once the terminator has been split off. `text::split_lines` reports both together.
    pub fn line(
        number: u32,
        document: Document,
        text: &str,
        ending: LineEnding,
        opts: &RenderOptions,
    ) -> Self {
        let (rendered, columns) = render_line(text, ending, opts);
        DisplayedRow {
            numbers: [Some(number), None],
            id: Some((document, number)),
            text: rendered.as_str().into(),
            columns: columns as u32,
            selectable: true,
            numbered: true,
            ..DisplayedRow::blank(RowClass::CONTEXT)
        }
    }

    /// A row of the given kind with nothing else set.
    ///
    /// The starting point for anything producing rows, so a provider states only the fields
    /// its content actually has and does not have to know what the rest default to.
    pub fn blank(class: RowClass) -> Self {
        DisplayedRow {
            class,
            numbered: false,
            full_width: false,
            numbers: [None; GUTTER_COLUMNS],
            id: None,
            source: None,
            text: SharedString::new(),
            columns: 0,
            gap: None,
            selectable: false,
            selectable_run: 0..0,
        }
    }
}

/// Which channel a range is painted in, which is what picks how it is drawn.
///
/// An open set. The view carries one style per channel and the numbers mean whatever the host
/// decides they mean, so a host can paint search results, review comments, or anything else it
/// has without this crate knowing those exist. Two numbers are spoken for, because this crate
/// produces ranges in them itself.
///
/// Channels are drawn in ascending order, so a higher number paints over a lower one. A channel
/// the view has no style for draws nothing rather than falling back to a colour that would
/// claim to mean something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Channel(pub u32);

impl Channel {
    /// What the user is selecting now.
    pub const SELECTION: Channel = Channel(0);
    /// Supplied from outside: a stored selection, or something a host is pointing at.
    pub const MARKED: Channel = Channel(1);

    /// The first channel this crate does not use itself.
    ///
    /// A host numbers its own from here rather than picking numbers, so that this crate taking
    /// another channel later does not silently collide with one already in use. Anything at or
    /// above this is the host's to define.
    pub const FIRST_FREE: Channel = Channel(2);
}

/// How much of one row a highlight covers, in the columns the grid is drawn on.
///
/// The same two cases as `SourceCharExtent`, converted through `map_span`. Kept as a separate
/// type rather than shared: these are the coordinates the view works in, and they move when
/// the tab width or the whitespace options change.
///
/// Columns, not pixels. Turning a column into a position on screen is a multiplication by the
/// character advance, and that happens in the markup; nothing in this crate knows a pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayColumnExtent {
    /// Columns `range` of the row.
    Columns(Range<u32>),
    /// From `from` to the edge of the pane, which is how a covered line ending is shown.
    ToEnd { from: u32 },
}

/// A range to paint behind one row's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    pub extent: DisplayColumnExtent,
    pub channel: Channel,
}
