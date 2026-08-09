//! Scrolling, and what holds still while it happens.
//!
//! Every assertion here stands for something that has broken before. The view scrolls on both
//! axes, the gutter is pinned against one of them, rows are drawn wider than the viewport, and
//! the scroll position is driven from outside so two panes can move together. Each of those
//! has silently stopped working at least once, and none of it is visible to a unit test.

// == Internal Crates
use fxv_diff_slint_tests::{first_element, harness, paired_harness, settle, show, show_paired};

/// Rows enough to overflow the 400px harness window several times over.
const LONG: u32 = 100;
/// Rows that fit in it with room to spare.
const SHORT: u32 = 5;

#[test]
fn a_document_shorter_than_the_view_cannot_scroll() {
    let h = paired_harness(SHORT);
    assert_eq!(h.get_max_scroll_y(), 0.0);
}

#[test]
fn the_vertical_range_grows_a_row_at_a_time() {
    // Stated as a difference so the test does not need to know the viewport height, which
    // changes when a horizontal scrollbar appears.
    let h = paired_harness(LONG);
    let long = h.get_max_scroll_y();
    show_paired(&h, LONG / 2);
    settle();
    let half = h.get_max_scroll_y();

    assert_eq!(long - half, (LONG / 2) as f32 * h.get_row_height());
}

#[test]
fn switching_to_a_shorter_document_pulls_the_view_back() {
    // The reported symptom: after scrolling to the end of a long file and switching to a short
    // one, nothing was drawn and it could not be scrolled back up, because the position was
    // never in range. This asserts the end state rather than which mechanism produced it; the
    // test below is the one that pins the view's own clamp.
    let h = harness(LONG);
    settle();
    h.set_scroll_y(-h.get_max_scroll_y());
    settle();
    assert!(
        h.get_scroll_y() < 0.0,
        "the long document should be scrollable"
    );

    show(&h, SHORT);
    settle();

    assert_eq!(h.get_max_scroll_y(), 0.0, "the short document fits");
    assert_eq!(h.get_scroll_y(), 0.0, "so the view must be back at the top");
}

#[test]
fn switching_to_a_document_that_still_scrolls_lands_in_range() {
    // This is the case the view's own clamp exists for, and removing that clamp fails here.
    // A document short enough to need no scrolling at all gets pulled back by other means, so
    // it does not discriminate; one that still scrolls, but less than before, does.
    let h = paired_harness(LONG);
    settle();
    h.set_scroll_y(-h.get_max_scroll_y());
    settle();

    show_paired(&h, LONG / 2);
    settle();

    let max = h.get_max_scroll_y();
    assert!(max > 0.0, "half of a long document should still scroll");
    assert!(
        h.get_scroll_y() >= -max && h.get_scroll_y() <= 0.0,
        "scroll position {} is outside 0..-{max}",
        h.get_scroll_y()
    );
}

#[test]
fn content_moves_sideways_while_the_gutter_stays_put() {
    // The gutter is drawn over the body and pinned against the visible edge, so line numbers
    // stay readable however far right the view has scrolled.
    let h = harness(20);
    settle();
    let gutter_before = first_element(&h, "RowView::i-gutter").absolute_position().x;
    let content_before = first_element(&h, "RowView::i-content")
        .absolute_position()
        .x;

    h.set_scroll_x(-100.0);
    settle();

    let gutter_after = first_element(&h, "RowView::i-gutter").absolute_position().x;
    let content_after = first_element(&h, "RowView::i-content")
        .absolute_position()
        .x;

    assert_eq!(gutter_before, gutter_after, "the gutter must not move");
    assert_eq!(
        content_before - content_after,
        100.0,
        "the content must move by exactly what it was scrolled"
    );
}

#[test]
fn a_row_is_drawn_across_the_whole_scrollable_width() {
    // A row sized to the viewport runs out the moment the view scrolls sideways, leaving
    // changed lines half tinted. It also must not be centred: a child wider than its parent is
    // centred by default, which shifts every line left by half the overflow.
    let h = harness(20);
    settle();
    let body = first_element(&h, "RowView::i-body");

    assert!(
        body.size().width >= h.get_scrollable_width(),
        "row body {} is narrower than the scrollable width {}",
        body.size().width,
        h.get_scrollable_width()
    );
    assert_eq!(
        body.absolute_position().x,
        0.0,
        "the row body must be anchored at the left edge, not centred"
    );
}

#[test]
fn the_rows_declare_a_horizontal_range_for_the_list_to_scroll() {
    // A ListView takes its scrollable width from the largest minimum width among its rows.
    // Assigning the width it should have is the one thing that does not work, so the range
    // existing at all is worth asserting.
    let h = harness(20);
    settle();
    assert!(
        h.get_scrollable_width() > h.get_visible_width(),
        "the fixture should be wider than the window, or this proves nothing"
    );

    h.set_scroll_x(-50.0);
    settle();
    let content = first_element(&h, "RowView::i-content")
        .absolute_position()
        .x;

    h.set_scroll_x(-150.0);
    settle();
    let moved = first_element(&h, "RowView::i-content")
        .absolute_position()
        .x;

    assert_eq!(content - moved, 100.0);
}
