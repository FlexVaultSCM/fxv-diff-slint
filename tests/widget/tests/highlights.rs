//! Painting a range of a line.
//!
//! The same layer draws whatever the user is selecting and whatever a host has asked to mark,
//! so these assert geometry rather than meaning: a range of columns becomes a rectangle at a
//! known place, because the grid is monospace and nothing wraps.

// == Internal Crates
use fxv_diff_slint::Channel;
use fxv_diff_slint_tests::{columns, harness, harness_with_highlights, settle, to_end};

/// Where every highlight on screen is drawn, as (x, width).
///
/// One element exists per highlight per row, so a view with highlights on one row yields
/// exactly that row's highlights however many rows are loaded.
fn painted(harness: &fxv_diff_slint_tests::Harness) -> Vec<(f32, f32)> {
    i_slint_backend_testing::ElementHandle::find_by_element_id(harness, "RowView::i-highlight")
        .map(|h| (h.absolute_position().x, h.size().width))
        .collect()
}

#[test]
fn a_row_with_no_highlights_paints_none() {
    let h = harness(10);
    settle();
    assert!(painted(&h).is_empty());
}

#[test]
fn a_highlight_covers_the_columns_it_names() {
    let h = harness_with_highlights(10, &[(Channel::SELECTION, vec![columns(0, 4..9)])]);
    settle();

    let advance = h.get_advance();
    let gutter = h.get_gutter_width();
    let drawn = painted(&h);

    assert_eq!(drawn.len(), 1, "one highlight, one rectangle");
    assert_eq!(drawn[0].0, gutter + 4.0 * advance, "starts at column 4");
    assert_eq!(drawn[0].1, 5.0 * advance, "and is five columns wide");
}

#[test]
fn several_highlights_on_one_row_are_all_drawn() {
    let h = harness_with_highlights(
        10,
        // Two channels over the same row: both are drawn, because setting one leaves the
        // other alone.
        &[
            (Channel::SELECTION, vec![columns(0, 0..3)]),
            (Channel::MARKED, vec![columns(0, 10..12)]),
        ],
    );
    settle();
    assert_eq!(painted(&h).len(), 2);
}

#[test]
fn a_highlight_covering_the_line_ending_reaches_the_pane_edge() {
    // A selection running onto the next line covers this line's ending, and the ending is
    // what the rest of the row stands for, so the paint does not stop at the last glyph.
    let h = harness_with_highlights(10, &[(Channel::SELECTION, vec![to_end(0, 4)])]);
    settle();

    let drawn = painted(&h);
    assert_eq!(drawn.len(), 1);

    let x = h.get_gutter_width() + 4.0 * h.get_advance();
    assert_eq!(drawn[0].0, x, "starts at column 4");
    assert_eq!(
        drawn[0].0 + drawn[0].1,
        h.get_scrollable_width(),
        "and runs to the far edge of the scrollable width, not the visible one"
    );
}

#[test]
fn a_highlight_sits_after_the_gutter() {
    // Column zero is the first character of the line, not the left edge of the row, so a
    // highlight starting there must clear the line numbers.
    let h = harness_with_highlights(10, &[(Channel::SELECTION, vec![columns(0, 0..2)])]);
    settle();

    let drawn = painted(&h);
    assert_eq!(drawn[0].0, h.get_gutter_width());
    assert!(h.get_gutter_width() > 0.0, "the fixture shows line numbers");
}

#[test]
fn the_default_style_paints_the_channels_this_crate_produces() {
    // A channel with no brush draws nothing, so an empty or short table would leave every
    // highlight invisible while the rectangles above still measure correctly.
    let h = harness(1);
    settle();
    assert!(
        h.get_styled_channels() > Channel::MARKED.0 as i32,
        "the default style should cover every channel the crate itself uses"
    );
}
