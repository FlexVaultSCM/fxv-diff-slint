//! Dragging out a selection.
//!
//! A drag is drawn from view properties rather than through the model, so these assert the
//! properties a drag leaves behind and the rectangles they produce. Nothing here resolves a
//! selection to file coordinates; that happens once, when the gesture ends.

// == External Crates
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

// == Internal Crates
use fxv_diff_slint::DiffPosition;
use fxv_diff_slint_tests::{harness, settle, Harness};

const ROWS: u32 = 30;

fn selectable(count: u32) -> Harness {
    let h = harness(count);
    h.set_selectable(true);
    settle();
    h
}

/// Every selection rectangle on screen, as (x, width).
fn painted(h: &Harness) -> Vec<(f32, f32)> {
    ElementHandle::find_by_element_id(h, "DiffRowView::i-selection")
        .map(|e| (e.absolute_position().x, e.size().width))
        .collect()
}

#[test]
fn nothing_is_selected_to_begin_with() {
    let h = selectable(ROWS);
    assert_eq!(h.get_selection_anchor().row, -1);
    assert!(painted(&h).is_empty());
}

#[test]
fn a_selection_over_one_row_covers_the_columns_between_its_ends() {
    let h = selectable(ROWS);
    h.set_selection_anchor(at_row(2, 4));
    h.set_selection_focus(at_row(2, 9));
    settle();

    let drawn = painted(&h);
    assert_eq!(drawn.len(), 1, "one row, one rectangle");
    assert_eq!(drawn[0].0, h.get_gutter_width() + 4.0 * h.get_advance());
    assert_eq!(drawn[0].1, 5.0 * h.get_advance());
}

#[test]
fn a_selection_made_upward_covers_the_same_columns() {
    // Anchor after focus is an ordinary selection, not an empty one: which end came first is
    // about extending it later, not about what is covered now.
    let h = selectable(ROWS);
    h.set_selection_anchor(at_row(2, 9));
    h.set_selection_focus(at_row(2, 4));
    settle();

    let drawn = painted(&h);
    assert_eq!(drawn.len(), 1);
    assert_eq!(drawn[0].0, h.get_gutter_width() + 4.0 * h.get_advance());
    assert_eq!(drawn[0].1, 5.0 * h.get_advance());
}

#[test]
fn a_middle_row_of_a_selection_is_covered_whole() {
    let h = selectable(ROWS);
    h.set_selection_anchor(at_row(1, 3));
    h.set_selection_focus(at_row(3, 2));
    settle();

    let drawn = painted(&h);
    assert_eq!(drawn.len(), 3, "first, middle and last row");

    // The middle row starts at column 0 and runs past the last character, because the line
    // ending is part of what was selected.
    let (x, width) = drawn[1];
    assert_eq!(x, h.get_gutter_width());
    assert!(
        width > h.get_visible_width(),
        "a covered line ending reaches the scrollable edge, not the last glyph"
    );
}

/// Presses at one point, moves through the rest, and releases.
///
/// Dispatched to the window rather than driven through a helper, because the helpers are async
/// and this harness runs with no event loop to drive them.
fn drag(h: &Harness, path: &[(f32, f32)]) {
    let (x, y) = path[0];
    h.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(x, y),
        button: PointerEventButton::Left,
    });
    settle();
    for (x, y) in &path[1..] {
        h.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(*x, *y),
        });
        settle();
    }
    let (x, y) = *path.last().unwrap();
    h.window().dispatch_event(WindowEvent::PointerReleased {
        position: LogicalPosition::new(x, y),
        button: PointerEventButton::Left,
    });
    settle();
}

/// A selection end, for setting one without a gesture.
fn at_row(row: i32, column: i32) -> DiffPosition {
    DiffPosition { row, column }
}

/// A point inside the first half of a column, in window coordinates.
///
/// The first half on purpose: a caret goes before the character the pointer is on or after it,
/// and the tipping point is the character's middle, so a point exactly halfway would be
/// asserting on a rounding decision rather than on the gesture.
fn at(h: &Harness, row: u32, column: u32) -> (f32, f32) {
    let x = h.get_gutter_width() + (column as f32 + 0.25) * h.get_advance();
    let y = (row as f32 + 0.5) * h.get_row_height();
    (x, y)
}

#[test]
fn a_drag_across_rows_sets_both_ends_and_reports_when_it_ends() {
    let h = selectable(ROWS);

    drag(&h, &[at(&h, 1, 3), at(&h, 2, 6), at(&h, 4, 8)]);

    assert_eq!(h.get_selection_anchor().row, 1, "where the drag began");
    assert_eq!(h.get_selection_anchor().column, 3);
    assert_eq!(h.get_selection_focus().row, 4, "where it ended");
    assert_eq!(h.get_selection_focus().column, 8);
    assert_eq!(h.get_finished_count(), 1, "reported once, on release");
}

#[test]
fn a_drag_reports_nothing_while_the_pointer_is_still_down() {
    let h = selectable(ROWS);
    let (x, y) = at(&h, 1, 3);
    h.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(x, y),
        button: PointerEventButton::Left,
    });
    settle();
    let (x, y) = at(&h, 3, 5);
    h.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
    settle();

    assert_eq!(
        h.get_selection_focus().row,
        3,
        "the focus follows the pointer"
    );
    assert_eq!(h.get_finished_count(), 0, "but nothing is resolved yet");
}

#[test]
fn the_far_half_of_a_character_puts_the_caret_after_it() {
    // What makes a click land where the pointer looks like it is, rather than always before
    // the character under it.
    let h = selectable(ROWS);
    let y = 1.5 * h.get_row_height();
    let x = h.get_gutter_width() + 3.75 * h.get_advance();
    drag(&h, &[(x, y)]);

    assert_eq!(h.get_selection_anchor().column, 4);
}

#[test]
fn a_pane_that_does_not_select_ignores_a_drag() {
    let h = harness(ROWS);
    settle();

    drag(&h, &[at(&h, 1, 3), at(&h, 4, 8)]);

    assert_eq!(h.get_selection_anchor().row, -1);
    assert_eq!(h.get_finished_count(), 0);
}

#[test]
fn a_drag_stops_at_a_gap_rather_than_crossing_it() {
    // A selection that spanned a gap would have to either omit the hidden lines when read
    // back, giving text that does not match what was selected, or fetch them, making reading a
    // selection have side effects. Refusing to cross avoids choosing between those.
    let h = fxv_diff_slint_tests::harness_with_gap(4, 6);
    h.set_selectable(true);
    settle();

    // Rows 0..3 are content, row 4 is the gap, rows 5.. are content again.
    drag(&h, &[at(&h, 1, 2), at(&h, 8, 2)]);

    assert_eq!(
        h.get_selection_focus().row,
        3,
        "clamped to the last row above the gap"
    );
}

#[test]
fn a_drag_cannot_begin_on_a_gap() {
    let h = fxv_diff_slint_tests::harness_with_gap(4, 6);
    h.set_selectable(true);
    settle();

    drag(&h, &[at(&h, 4, 2), at(&h, 6, 2)]);

    assert_eq!(h.get_selection_anchor().row, -1, "a gap starts nothing");
}

#[test]
fn showing_different_rows_drops_the_selection() {
    // A selection names rows by index, so it means nothing against a different document.
    // Leaving it would draw over whatever now sits at those indices.
    let h = selectable(ROWS);
    drag(&h, &[at(&h, 1, 2), at(&h, 3, 5)]);
    assert_eq!(h.get_selection_anchor().row, 1, "selected to begin with");

    fxv_diff_slint_tests::show(&h, ROWS + 5);
    settle();

    assert_eq!(h.get_selection_anchor().row, -1);
    assert_eq!(h.get_selection_focus().row, -1);
    assert!(painted(&h).is_empty(), "and nothing is drawn");
}
