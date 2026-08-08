//! Bringing a row into sight.
//!
//! The view owns this arithmetic because it owns the row height, the visible height and the
//! scroll range. A revealed row is centred, so stepping through matches keeps each one in the
//! same place on screen with its surroundings visible either side.

// == Internal Crates
use fxv_diff_slint_tests::{harness, paired_harness, settle, Harness};

const ROWS: u32 = 200;

/// The viewport's height, derived from the scroll range rather than measured separately.
///
/// A view can be scrolled by its whole content less what is on screen, so the part on screen
/// is the difference. Taking it from the range means the two cannot disagree.
fn visible_height(h: &Harness) -> f32 {
    ROWS as f32 * h.get_row_height() - h.get_max_scroll_y()
}

/// Where the middle of a row sits once the view has settled.
fn row_centre(h: &Harness, row: i32) -> f32 {
    row as f32 * h.get_row_height() + h.get_row_height() / 2.0 + h.get_scroll_y()
}

#[test]
fn a_revealed_row_is_centred() {
    let h = harness(ROWS);
    settle();

    h.set_reveal_row(120);
    settle();

    assert_eq!(row_centre(&h, 120), visible_height(&h) / 2.0);
}

#[test]
fn a_row_behind_the_viewport_is_centred_too() {
    // Coming back up centres just the same, so stepping backwards through matches behaves
    // like stepping forwards.
    let h = harness(ROWS);
    settle();
    h.set_scroll_y(-150.0 * h.get_row_height());
    settle();

    h.set_reveal_row(20);
    settle();

    assert_eq!(row_centre(&h, 20), visible_height(&h) / 2.0);
}

#[test]
fn a_row_near_the_top_settles_short_of_the_middle() {
    // There is nothing above row 0 to scroll into view, so the first screenful cannot centre.
    let h = harness(ROWS);
    settle();
    h.set_scroll_y(-150.0 * h.get_row_height());
    settle();

    h.set_reveal_row(0);
    settle();

    assert_eq!(h.get_scroll_y(), 0.0, "the top, not above it");
}

#[test]
fn revealing_the_last_row_does_not_scroll_past_the_end() {
    let h = harness(ROWS);
    settle();

    h.set_reveal_row(ROWS as i32 - 1);
    settle();

    assert_eq!(h.get_scroll_y(), -h.get_max_scroll_y());
}

#[test]
fn revealing_nothing_leaves_the_view_alone() {
    let h = harness(ROWS);
    settle();
    h.set_scroll_y(-50.0 * h.get_row_height());
    settle();

    h.set_reveal_row(-1);
    settle();

    assert_eq!(h.get_scroll_y(), -50.0 * h.get_row_height());
}

#[test]
fn the_token_reveals_a_row_that_is_already_the_one_asked_for() {
    // Asking for the same match again is a real request: the view has been scrolled away from
    // it since, and the row alone cannot say that anything changed.
    let h = harness(ROWS);
    settle();
    h.set_reveal_row(120);
    settle();
    let centred = h.get_scroll_y();

    h.set_scroll_y(-5.0 * h.get_row_height());
    settle();
    h.set_reveal_row(120);
    settle();
    assert_ne!(h.get_scroll_y(), centred, "the row alone changed nothing");

    h.set_reveal_token(1);
    settle();

    assert_eq!(h.get_scroll_y(), centred, "the token brought it back");
}

#[test]
fn revealing_a_row_that_is_not_there_stays_within_the_content() {
    // A host can hold a row index the view has since outgrown, from a match list built before
    // a gap closed. Scrolling to where that row would have been shows nothing at all, and
    // cannot be scrolled back from.
    //
    // Asserted on two views sharing one position, because that is the only arrangement where
    // it can fail: a single view's list pulls its own position back into range, and stops
    // doing so exactly when a second element binds the property.
    let h = paired_harness(ROWS);
    settle();

    h.set_reveal_row(10_000);
    settle();

    assert_eq!(h.get_scroll_y(), -h.get_max_scroll_y());
}
