//! Scrolling a pane with the wheel or a trackpad.
//!
//! Asserted for both panes, because they are separate components with their own lists: a fix
//! to one is not a fix to the other, and this is the sort of thing that only shows up by hand.

// == External Crates
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

// == Internal Crates
use fxv_diff_slint_tests::{code_harness, harness, settle};

const ROWS: u32 = 200;

#[test]
fn the_diff_pane_scrolls_on_a_wheel_event() {
    let h = harness(ROWS);
    settle();
    h.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(200.0, 200.0),
        delta_x: 0.0,
        delta_y: -120.0,
    });
    settle();

    assert!(h.get_scroll_y() < 0.0, "moved down");
}

#[test]
fn the_plain_pane_scrolls_on_a_wheel_event() {
    let h = code_harness(ROWS);
    settle();
    h.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(200.0, 200.0),
        delta_x: 0.0,
        delta_y: -120.0,
    });
    settle();

    assert!(h.get_scroll_y() < 0.0, "moved down");
}

#[test]
fn a_selectable_pane_still_scrolls_on_a_wheel_event() {
    // The rows carry a touch area for the drag, and anything inside a list competes with the
    // list for pointer events. Turning selection on must not cost the wheel.
    let h = code_harness(ROWS);
    h.set_selectable(true);
    settle();
    h.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(200.0, 200.0),
        delta_x: 0.0,
        delta_y: -120.0,
    });
    settle();

    assert!(h.get_scroll_y() < 0.0, "moved down");
}

#[test]
fn the_plain_pane_scrolls_sideways() {
    // A ListView takes its scrollable width from the largest minimum width among its rows, and
    // that minimum is declared by a spacer child of the row. Lose the spacer and the pane still
    // draws correctly, still scrolls vertically, and simply has no horizontal range at all.
    let h = code_harness(ROWS);
    settle();
    assert!(
        h.get_scrollable_width() > h.get_visible_width(),
        "the fixture should be wider than the window: scrollable {} visible {}",
        h.get_scrollable_width(),
        h.get_visible_width()
    );

    h.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(200.0, 200.0),
        delta_x: -120.0,
        delta_y: 0.0,
    });
    settle();

    assert!(h.get_scroll_x() < 0.0, "moved right");
}
