//! What the gutter actually draws.
//!
//! A row carries its numbers in a fixed set of slots and the gutter draws as many columns as it
//! was told to, filling them from the first slot. Getting that correspondence wrong leaves a
//! pane with no line numbers at all, which is invisible to any test that inspects the rows
//! rather than what came out of them.

// == Internal Crates
use fxv_diff_slint_tests::{
    harness, harness_with_columns, harness_with_rows, settle, two_numbered_rows, Harness,
};

/// Every number the gutter is showing, across both of its columns.
///
/// Two queries because the columns are separate elements and Slint ids are unique within a
/// component, so they cannot share one.
fn numbers(harness: &Harness) -> Vec<String> {
    ["RowView::i-number", "RowView::i-number-2"]
        .into_iter()
        .flat_map(|id| i_slint_backend_testing::ElementHandle::find_by_element_id(harness, id))
        .filter_map(|e| e.accessible_label().map(|l| l.to_string()))
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn a_pane_told_to_draw_one_column_still_shows_its_numbers() {
    // The bug this exists for: a one-column pane puts each row's number in the first slot, and
    // a gutter reading the second slot drew nothing at all.
    let h = harness_with_columns(10, 1);
    settle();

    assert!(
        !numbers(&h).is_empty(),
        "a pane showing one file must still number its lines"
    );
}

/// Rows enough to fit on screen at once, so every one of them is instantiated.
const ROWS: u32 = 10;

#[test]
fn one_column_draws_one_number_per_row() {
    let h = harness_with_rows(two_numbered_rows(ROWS), 1);
    settle();

    assert_eq!(numbers(&h).len(), ROWS as usize);
}

#[test]
fn a_second_column_draws_the_row_second_number() {
    // Rows with a number in both slots, as an unchanged line of a diff has.
    let h = harness_with_rows(two_numbered_rows(ROWS), 2);
    settle();

    assert_eq!(
        numbers(&h).len(),
        2 * ROWS as usize,
        "both slots drawn, once per row"
    );
}

#[test]
fn the_gutter_narrows_when_it_carries_one_column() {
    // Widths are properties rather than elements, so both harnesses can be read. Element
    // queries cannot: only the most recently created window answers them.
    let two = harness(ROWS);
    settle();
    let wide = two.get_gutter_width();

    let one = harness_with_columns(ROWS, 1);
    settle();

    assert!(
        one.get_gutter_width() < wide,
        "one column is narrower than two"
    );
}
