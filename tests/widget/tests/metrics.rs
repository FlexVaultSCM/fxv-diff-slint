//! The character grid every other position in the view is derived from.
//!
//! These assert exact pixel values. That is deliberate: the font is embedded precisely so the
//! grid is identical everywhere, and a change here means either the font or the font size
//! changed, which is never accidental.

// == Internal Crates
use fxv_diff_slint_tests::{harness, COLUMNS};

/// DejaVu Sans Mono at 13px advances 7.827px and lays out lines 15.13px apart. Both are
/// rounded up, because the grid has to sit on whole pixels.
const ADVANCE: f32 = 8.0;
const ROW_HEIGHT: f32 = 16.0;
/// CodeStyle.gutter-padding.
const PADDING: f32 = 8.0;

#[test]
fn the_grid_is_the_bundled_fonts_cell() {
    let h = harness(20);
    assert_eq!(h.get_advance(), ADVANCE);
    assert_eq!(h.get_row_height(), ROW_HEIGHT);
}

#[test]
fn the_row_height_sits_on_whole_pixels() {
    // A fractional row height puts the scroll arithmetic out of step with what is drawn: a row
    // is at least as tall as the Text inside it, and that Text rounds its own height up.
    let h = harness(20);
    let row_height = h.get_row_height();
    assert_eq!(row_height, row_height.round());
}

#[test]
fn the_gutter_holds_a_four_digit_column_per_side() {
    // Two columns of four digits, with padding outside each and between them.
    let h = harness(20);
    assert_eq!(h.get_gutter_width(), 2.0 * 4.0 * ADVANCE + 3.0 * PADDING);
}

#[test]
fn the_scrollable_width_is_the_gutter_plus_the_content() {
    let h = harness(20);
    assert_eq!(h.get_content_width(), COLUMNS as f32 * ADVANCE);
    assert_eq!(
        h.get_scrollable_width(),
        h.get_gutter_width() + h.get_content_width()
    );
}
