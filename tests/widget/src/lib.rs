//! Widget-level tests for the diff view.
//!
//! Everything that can be checked without a window lives in `fxv-diff-slint`'s own unit tests.
//! What is left is layout and interaction: scroll ranges, what stays put when the view moves,
//! and what a click on a gap asks for. Those need a running component, so they live here.

// == Std
use std::time;

// == External Crates
use slint::SharedString;

// == Internal Crates
use fxv_diff_slint::{GapState, Row, RowKind, Rows, ViewRows};

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

/// A harness with `count` plain context rows.
///
/// Rows are built here rather than parsed from a diff because these tests care about how many
/// there are and how wide they are, not what they say. Row flattening has its own tests.
///
/// Each test gets its own backend. `init_no_event_loop` sets a per-thread platform and panics
/// if one is already set, so it is called once per test rather than once per process.
pub fn harness(count: u32) -> Harness {
    i_slint_backend_testing::init_no_event_loop();
    let harness = Harness::new().expect("creating the harness window");
    show(&harness, count);
    harness
}

/// Replaces what the harness is showing, as switching to another file does.
pub fn show(harness: &Harness, count: u32) {
    let view = ViewRows::from(&context_rows(count));
    harness.set_longest_line_columns(view.longest_line_columns);
    harness.set_rows(view.rows);
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
    let view = ViewRows::from(&context_rows(count));
    harness.set_longest_line_columns(view.longest_line_columns);
    harness.set_rows(view.rows);
}

/// Rows handed through the library's own conversion rather than a copy of it, so a break in
/// that conversion shows up here rather than being papered over.
fn context_rows(count: u32) -> Rows {
    let text: SharedString = "x".repeat(COLUMNS as usize).into();
    Rows {
        rows: (1..=count)
            .map(|n| Row {
                kind: RowKind::Context,
                left_line: Some(n),
                right_line: Some(n),
                text: text.clone(),
                hidden_count: 0,
                gap_state: GapState::Hidden,
                columns: COLUMNS,
                source: None,
            })
            .collect(),
        longest_line_columns: COLUMNS,
    }
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
