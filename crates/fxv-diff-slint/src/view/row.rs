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
use crate::span::Side;
use crate::text::LineEnding;
use crate::text::{render_line, RenderOptions};

/// How many numbers a gutter can show at once.
///
/// Two, because that is what an inline diff needs and nothing here wants more. The cap lives
/// at this level on purpose: a layout has no opinion about gutters, and the widget struct
/// carries two plain integers rather than a list, which would cost every row an allocation to
/// hold what fits in eight bytes.
pub const GUTTER_COLUMNS: usize = 2;

/// What a row draws as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Names the file.
    Header,
    /// Unchanged content, shown to give the change context.
    Context,
    Added,
    Removed,
    /// Content that exists but is not shown.
    Gap,
    /// Nothing on this side. Keeps the two panes of a split view in step.
    Filler,
}

/// One row as a pane will draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayedRow {
    pub kind: RowKind,
    /// Gutter numbers, in the order the view draws them. A pane showing one file fills only
    /// the first.
    pub numbers: [Option<u32>; GUTTER_COLUMNS],
    /// Which file names this row and the number it has there, for describing a selection over
    /// it. Absent on rows that stand for no line.
    ///
    /// Settled here rather than asked for later, because a pane knows which file it is showing
    /// and an entry knows which side a line came from.
    pub id: Option<(Side, u32)>,
    /// The line this row was rendered from, for anything needing the original text.
    pub source: Option<LineRef>,
    /// Display text, tabs already expanded. Empty for fillers and gaps.
    pub text: SharedString,
    /// Columns the text occupies.
    pub columns: u32,
    /// How many lines a gap is hiding. Zero for every other kind.
    pub hidden_count: u32,
    /// Only meaningful on a gap.
    pub gap_state: GapState,
    /// A gap's hunk heading, or why its fetch failed. Empty otherwise.
    pub note: SharedString,
    /// Where a gap's hidden run starts, on each side.
    pub gap_start: (u32, u32),
    /// The run of a gap being fetched, numbered on the right, when one is.
    pub pending: Option<(u32, u32)>,
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
        side: Side,
        text: &str,
        ending: LineEnding,
        opts: &RenderOptions,
    ) -> Self {
        let (rendered, columns) = render_line(text, ending, opts);
        DisplayedRow {
            numbers: [Some(number), None],
            id: Some((side, number)),
            text: rendered.as_str().into(),
            columns: columns as u32,
            selectable: true,
            ..DisplayedRow::blank(RowKind::Context)
        }
    }

    /// A row of the given kind with nothing else set.
    ///
    /// The starting point for anything producing rows, so a provider states only the fields
    /// its content actually has and does not have to know what the rest default to.
    pub fn blank(kind: RowKind) -> Self {
        DisplayedRow {
            kind,
            numbers: [None; GUTTER_COLUMNS],
            id: None,
            source: None,
            text: SharedString::new(),
            columns: 0,
            hidden_count: 0,
            gap_state: GapState::Hidden,
            note: SharedString::new(),
            gap_start: (0, 0),
            pending: None,
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
