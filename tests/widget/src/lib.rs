//! Widget-level tests for the diff view.
//!
//! Everything that can be checked without a window lives in `fxv-diff-slint`'s own unit tests.
//! What is left is layout and interaction: scroll ranges, what stays put when the view moves,
//! and what a click on a gap asks for. Those need a running component, so they live here.

// == Std
use std::time;

// == Std
use std::cell::Cell;
use std::ops::Range;

// == External Crates

// == Internal Crates
use fxv_diff_slint::{
    Channel, DisplayColumnExtent, DisplayedRow, LineEnding, RenderOptions, RowKind, RowModel, Side,
};

// Machine-generated. `dead_code` because a consumer re-parses the library's .slint sources and
// re-embeds the images they reference, while the globals holding them come from the library
// crate, leaving the duplicates unread.
#[allow(clippy::absolute_paths, dead_code)]
mod ui {
    slint::include_modules!();
}

pub use ui::{Harness, PairedHarness};

/// Columns the generated rows occupy. Wide enough that the content overflows the harness
/// window, so there is something to scroll sideways.
pub const COLUMNS: u32 = 200;

thread_local! {
    static BACKEND: Cell<bool> = const { Cell::new(false) };
}

/// Starts the testing backend, once per thread.
///
/// It sets a per-thread platform and panics if one is already set, so a test wanting two
/// harnesses to compare them cannot simply call it twice.
fn backend() {
    BACKEND.with(|started| {
        if !started.replace(true) {
            i_slint_backend_testing::init_no_event_loop();
        }
    });
}

/// A harness with `count` plain context rows.
///
/// Rows are built here rather than parsed from a diff because these tests care about how many
/// there are and how wide they are, not what they say. Row flattening has its own tests.
///
/// Each test gets its own backend. `init_no_event_loop` sets a per-thread platform and panics
/// if one is already set, so it is called once per test rather than once per process.
pub fn harness(count: u32) -> Harness {
    backend();
    let harness = Harness::new().expect("creating the harness window");
    show(&harness, count);
    harness
}

/// A harness whose gutter draws `columns` number columns.
pub fn harness_with_columns(count: u32, columns: i32) -> Harness {
    harness_with_rows(context_rows(count), columns)
}

/// A harness over rows supplied directly, for cases the plain fixture cannot express.
pub fn harness_with_rows(rows: Vec<DisplayedRow>, columns: i32) -> Harness {
    backend();
    let harness = Harness::new().expect("creating the harness window");
    harness.set_gutter_columns(columns);
    let view = RowModel::from_rows(rows);
    harness.set_longest_line_columns(view.longest_line_columns());
    harness.set_rows(view.model());
    harness
}

/// Rows carrying a number in both gutter slots, as an unchanged line of a diff does.
pub fn two_numbered_rows(count: u32) -> Vec<DisplayedRow> {
    context_rows(count)
        .into_iter()
        .enumerate()
        .map(|(i, mut row)| {
            row.numbers[1] = Some(i as u32 + 100);
            row
        })
        .collect()
}

/// Replaces what the harness is showing, as switching to another file does.
pub fn show(harness: &Harness, count: u32) {
    let view = RowModel::from_rows(context_rows(count));
    harness.set_longest_line_columns(view.longest_line_columns());
    harness.set_rows(view.model());
}

/// Two panes sharing one scroll position, which is the arrangement that stops the list
/// correcting that position on its own. See the note on `PairedHarness`.
pub fn paired_harness(count: u32) -> PairedHarness {
    i_slint_backend_testing::init_no_event_loop();
    let harness = PairedHarness::new().expect("creating the harness window");
    show_paired(&harness, count);
    harness
}

/// Replaces what a paired harness is showing.
pub fn show_paired(harness: &PairedHarness, count: u32) {
    let view = RowModel::from_rows(context_rows(count));
    harness.set_longest_line_columns(view.longest_line_columns());
    harness.set_rows(view.model());
}

/// Rows handed through the library's own conversion rather than a copy of it, so a break in
/// that conversion shows up here rather than being papered over.
pub fn context_rows(count: u32) -> Vec<DisplayedRow> {
    let text = "x".repeat(COLUMNS as usize);
    let opts = RenderOptions::default();
    (1..=count)
        .map(|n| DisplayedRow::line(n, Side::Right, &text, LineEnding::Lf, &opts))
        .collect()
}

/// A harness showing rows with a gap partway down.
///
/// A gap is not selectable, so it separates one run of rows from the next. Anything about a
/// selection meeting that boundary needs a document that has one.
pub fn harness_with_gap(before: u32, after: u32) -> Harness {
    backend();
    let harness = Harness::new().expect("creating the harness window");
    let mut rows = context_rows(before);
    rows.push(DisplayedRow::blank(RowKind::Gap));
    rows.extend(context_rows(after));

    let view = RowModel::from_rows(rows);
    harness.set_longest_line_columns(view.longest_line_columns());
    harness.set_rows(view.model());
    harness
}

/// A harness whose rows carry the given ranges, for asserting where they are painted.
///
/// Takes one entry per channel, because that is how the view is driven: a channel is set as a
/// whole and replaces whatever it was painting before.
pub fn harness_with_highlights(
    count: u32,
    channels: &[(Channel, Vec<(usize, DisplayColumnExtent)>)],
) -> Harness {
    backend();
    let harness = Harness::new().expect("creating the harness window");
    let mut view = RowModel::from_rows(context_rows(count));
    for (channel, ranges) in channels {
        view.set_channel(*channel, ranges);
    }
    harness.set_longest_line_columns(view.longest_line_columns());
    harness.set_rows(view.model());
    harness
}

/// A range covering some columns of one row.
pub fn columns(row: usize, columns: Range<u32>) -> (usize, DisplayColumnExtent) {
    (row, DisplayColumnExtent::Columns(columns))
}

/// A range running from a column to the edge of the pane, as a covered line ending draws.
pub fn to_end(row: usize, from: u32) -> (usize, DisplayColumnExtent) {
    (row, DisplayColumnExtent::ToEnd { from })
}

/// Rows currently on screen, for asserting what a viewport shows.
pub fn row_count(harness: &Harness) -> usize {
    use slint::Model;
    harness.get_rows().row_count()
}

/// Lets deferred work land.
///
/// A `changed` callback runs after the code that triggered it returns, so a correction made by
/// one is not visible on the line after the setter that caused it. Advancing the mocked clock
/// by nothing runs the change handlers and re-instantiates whatever they dirtied, which is the
/// same point in the frame a running application would reach on its own.
pub fn settle() {
    i_slint_backend_testing::mock_elapsed_time(time::Duration::ZERO);
}

/// The first instance of a row element, by its qualified id.
///
/// The list virtualizes, so several exist and any of them answers the question these tests
/// ask, which is where an element sits relative to the view rather than which row it is.
pub fn first_element(harness: &Harness, id: &str) -> i_slint_backend_testing::ElementHandle {
    i_slint_backend_testing::ElementHandle::find_by_element_id(harness, id)
        .next()
        .unwrap_or_else(|| {
            panic!("no element with id {id}; queries by id need the debug info build.rs enables")
        })
}
