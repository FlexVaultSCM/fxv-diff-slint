//! What the gutter actually draws.
//!
//! A row carries its numbers in a fixed set of slots and the gutter draws as many columns as it
//! was told to, filling them from the first slot. Getting that correspondence wrong leaves a
//! pane with no line numbers at all, which is invisible to any test that inspects the rows
//! rather than what came out of them.

// == Internal Crates
use fxv_diff_slint_tests::{
    harness, harness_with_columns, harness_with_gap, harness_with_rows, settle, two_numbered_rows,
    Harness,
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

#[test]
fn the_row_classes_agree_across_the_boundary() {
    // A class number is written down three times: as a Rust constant, as a name in the markup,
    // and as a position in the style table. Nothing makes them agree, and a mismatch would show
    // up as rows drawn in the wrong colour or a gap that stops being recognised as one, both a
    // long way from the cause.
    use fxv_diff_slint::RowClass;
    use slint::Model;

    let h = harness(1);
    settle();

    let named: Vec<i32> = h.get_class_numbers().iter().collect();
    let expected = [
        RowClass::CONTEXT,
        RowClass::ADDED,
        RowClass::REMOVED,
        RowClass::GAP,
        RowClass::FILLER,
        RowClass::HEADER,
        RowClass::FIRST_FREE,
    ];
    for (named, class) in named.iter().zip(expected) {
        assert_eq!(
            *named, class.0 as i32,
            "{class:?} is numbered differently in the markup"
        );
    }

    assert_eq!(
        h.get_styled_classes(),
        RowClass::FIRST_FREE.0 as i32,
        "the style table should cover every class this crate produces, and no more"
    );
}

#[test]
fn a_gap_shows_its_heading_beside_the_count() {
    // A gap says what it stands for in a band of its own, from `note` rather than `text`: the
    // pane draws `text` for every row, so a gap putting its heading there would have it drawn
    // twice, once by the pane and once by the band.
    let h = harness_with_gap(3, 3);
    settle();

    let shown: Vec<String> =
        i_slint_backend_testing::ElementHandle::find_by_element_id(&h, "RowView::i-gap-text")
            .filter_map(|e| e.accessible_label().map(|l| l.to_string()))
            .collect();

    assert!(
        shown.iter().any(|t| t == "12 hidden    fn thing()"),
        "the band should say how much is hidden and what follows, got {shown:?}"
    );
}
