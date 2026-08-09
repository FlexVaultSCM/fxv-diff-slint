//! The gesture, and turning it into file coordinates.
//!
//! A selection is made in view coordinates: a row on screen and a column across it. It is
//! reported in file coordinates, as `LineSpan`s, which is the form a host stores. This module
//! owns that translation and the rules about where a selection may go.
//!
//! It holds the input side of the pipeline. `highlight` holds the output side, and `span` the
//! durable form in between.
//!
//! WIP: nothing drives any of this yet. There is no pointer handling, so a `Selection` can
//! only be built by hand. The arithmetic and the rules are here and tested; the gestures that
//! would produce them are not written.
//!
//! Two decisions already made about what comes next, recorded so they are not rediscovered.
//! A live drag will not travel through here at all: the view will carry the two carets as
//! plain properties and each visible row will work out its own rectangle, so that moving the
//! pointer costs no spans, no lookups and no model writes. `to_spans` runs when the gesture
//! ends, which is when the durable form is actually wanted. And word boundaries, for
//! double-click, belong with whatever tokenizes a line for syntax highlighting rather than
//! being a second opinion about where a word starts.

// == Std
use std::ops::Range;

// == Internal Crates
use crate::diff::model::FileDiff;
use crate::span::{LineSpan, SourceCharExtent};
use crate::text::{source_index_at, RenderOptions};
use crate::ui;
use crate::view::DisplayedRow;

/// One end of a selection, in the coordinates the view works in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub row: usize,
    /// Display column, because that is what a pointer position converts to directly. The
    /// conversion to a source offset happens once, in `to_spans`.
    pub column: u32,
}

impl From<ui::PanePosition> for Caret {
    /// Takes a position from the widget, which reports one in the same coordinates.
    ///
    /// Negative values cannot be a row or a column, so they clamp to zero rather than wrap.
    /// The widget uses a row of -1 to mean no selection at all, and a caller checks for that
    /// before converting rather than being handed a caret at row zero.
    fn from(at: ui::PanePosition) -> Self {
        Caret {
            row: at.row.max(0) as usize,
            column: at.column.max(0) as u32,
        }
    }
}

impl From<ui::PaneSelection> for Selection {
    fn from(selection: ui::PaneSelection) -> Self {
        Selection {
            anchor: selection.anchor.into(),
            focus: selection.focus.into(),
        }
    }
}

/// A selection in progress, or a finished one.
///
/// Two points rather than a region: anchor is where the drag started and focus is where it is
/// now, so a selection made upward has its focus before its anchor. That distinction is kept
/// because extending with shift has to grow from the anchor, not from whichever end happens to
/// come first on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Caret,
    pub focus: Caret,
}

impl Selection {
    pub fn at(caret: Caret) -> Self {
        Selection {
            anchor: caret,
            focus: caret,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The two ends in the order they appear on screen.
    fn ordered(&self) -> (Caret, Caret) {
        let backwards = (self.focus.row, self.focus.column) < (self.anchor.row, self.anchor.column);
        if backwards {
            (self.focus, self.anchor)
        } else {
            (self.anchor, self.focus)
        }
    }
}

/// The stretch of rows a selection starting at `row` may cover.
///
/// A selection is confined to one such stretch. Crossing a gap would mean either dropping the
/// hidden lines when the selection was resolved, so the reported range would not match what was
/// highlighted, or fetching them, which would make selecting have side effects. Refusing to
/// cross avoids choosing between those. Expand the gap first, then select.
pub fn run_bounds(rows: &[DisplayedRow], row: usize) -> Range<usize> {
    if rows.get(row).is_none_or(|r| !r.selectable) {
        return row..row;
    }

    let start = rows[..row]
        .iter()
        .rposition(|r| !r.selectable)
        .map_or(0, |i| i + 1);
    let end = rows[row..]
        .iter()
        .position(|r| !r.selectable)
        .map_or(rows.len(), |i| row + i);

    start..end
}

/// Pulls a target row back into the run the selection started in.
pub fn clamp_to_run(rows: &[DisplayedRow], anchor_row: usize, target: usize) -> usize {
    let run = run_bounds(rows, anchor_row);
    if run.is_empty() {
        return anchor_row;
    }
    target.clamp(run.start, run.end - 1)
}

/// The file coordinates a selection covers.
///
/// Rows that stand for no line contribute nothing, so a selection dragged across a filler
/// simply skips it rather than reporting an empty range nobody can act on.
pub fn to_spans(
    rows: &[DisplayedRow],
    file: &FileDiff,
    opts: &RenderOptions,
    selection: &Selection,
) -> Vec<LineSpan> {
    if selection.is_empty() {
        return Vec::new();
    }
    let (start, end) = selection.ordered();

    let mut spans = Vec::new();
    let last = end.row.min(rows.len().saturating_sub(1));
    for (index, row) in rows.iter().enumerate().take(last + 1).skip(start.row) {
        let (Some(source), Some((side, line))) = (row.source, row.id) else {
            continue;
        };
        let Some(text) = file.line(source).map(|l| l.text.as_str()) else {
            continue;
        };

        // Only the first row is cut at a caret on its left. Only the last is cut on its
        // right; every earlier row runs off the end of its line and takes the line ending
        // with it, which is what a selection spanning several lines actually covers.
        let from = if index == start.row {
            source_index_at(text, start.column as usize, opts) as u32
        } else {
            0
        };

        let extent = if index == end.row {
            let to = source_index_at(text, end.column as usize, opts) as u32;
            if from >= to {
                continue;
            }
            SourceCharExtent::Columns(from..to)
        } else {
            SourceCharExtent::ToEnd { from }
        };

        spans.push(LineSpan { side, line, extent });
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::render::Pane;
    use crate::span::Side;
    use crate::test_fixtures::{file, removed_row, rows, shown, split};
    use crate::view::RowKind;

    fn caret(row: usize, column: u32) -> Caret {
        Caret { row, column }
    }

    #[test]
    fn a_selection_within_one_line_reports_that_line() {
        let f = file();
        let r = rows(&f);
        let row = removed_row(&r);
        let selection = Selection {
            anchor: caret(row, 4),
            focus: caret(row, 7),
        };

        let spans = to_spans(&r, &f, &RenderOptions::default(), &selection);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].side, Side::Left);
        assert_eq!(spans[0].line, 11);
    }

    #[test]
    fn a_tab_makes_display_columns_and_source_offsets_differ() {
        // The tab occupies four columns and one character, so a selection starting at display
        // column 4 starts at source character 1. Reporting the column instead would put a
        // stored anchor three characters late, and moving the tab width would move it again.
        let f = file();
        let r = rows(&f);
        let row = removed_row(&r);
        let selection = Selection {
            anchor: caret(row, 4),
            focus: caret(row, 7),
        };

        let spans = to_spans(&r, &f, &RenderOptions::default(), &selection);
        assert_eq!(
            spans[0].extent,
            SourceCharExtent::Columns(1..4),
            "source characters, not columns"
        );
    }

    #[test]
    fn a_selection_stops_at_a_gap() {
        let f = file();
        let r = rows(&f);
        // The fixture starts at line 10, so a leading gap row precedes the content.
        let gap = r
            .iter()
            .position(|row| row.kind == RowKind::Gap)
            .expect("the fixture hides the lines before the hunk");
        let first_content = gap + 1;

        let run = run_bounds(&r, first_content);
        assert_eq!(run.start, first_content, "the run begins after the gap");
        assert_eq!(
            clamp_to_run(&r, first_content, 0),
            first_content,
            "dragging back over the gap stops at it"
        );
    }

    #[test]
    fn a_selection_cannot_start_on_a_gap() {
        let f = file();
        let r = rows(&f);
        let gap = r.iter().position(|row| row.kind == RowKind::Gap).unwrap();
        assert!(run_bounds(&r, gap).is_empty());
    }

    #[test]
    fn a_pane_names_unchanged_lines_by_its_own_side() {
        let f = file();
        let layout = split(&f);
        let left = shown(&layout, &f, Pane::Left);
        let opts = RenderOptions::default();

        let context = left
            .iter()
            .position(|r| r.kind == RowKind::Context)
            .unwrap();
        let spans = to_spans(
            &left,
            &f,
            &opts,
            &Selection {
                anchor: caret(context, 0),
                focus: caret(context, 5),
            },
        );

        assert_eq!(
            spans[0].side,
            Side::Left,
            "an unchanged line selected in the left pane is about the left file"
        );
    }

    #[test]
    fn an_unchanged_line_is_named_by_the_right_file_inline() {
        let f = file();
        let r = rows(&f);
        let context = r
            .iter()
            .position(|row| row.kind == RowKind::Context)
            .unwrap();
        let spans = to_spans(
            &r,
            &f,
            &RenderOptions::default(),
            &Selection {
                anchor: caret(context, 0),
                focus: caret(context, 5),
            },
        );

        assert_eq!(spans[0].side, Side::Right);
    }

    #[test]
    fn selecting_across_the_two_sides_reports_both() {
        // Inline puts removals above the additions that replaced them, so a drag through both
        // crosses from the left file to the right. This is why a selection is a list of lines
        // rather than a start and an end.
        let f = file();
        let r = rows(&f);
        let removed = r
            .iter()
            .position(|row| row.kind == RowKind::Removed)
            .unwrap();
        let added = r.iter().position(|row| row.kind == RowKind::Added).unwrap();

        let spans = to_spans(
            &r,
            &f,
            &RenderOptions::default(),
            &Selection {
                anchor: caret(removed, 2),
                focus: caret(added, 9),
            },
        );

        let sides: Vec<Side> = spans.iter().map(|s| s.side).collect();
        assert_eq!(sides, vec![Side::Left, Side::Right]);
    }

    #[test]
    fn the_rows_between_the_two_ends_are_taken_whole() {
        // Only the first and last rows are cut at a caret. A four row selection has two in the
        // middle, and each of those must run the full length of its line whatever column the
        // carets happen to sit at.
        let f = file();
        let r = rows(&f);
        let first = r
            .iter()
            .position(|row| row.kind == RowKind::Context)
            .unwrap();
        let last = r
            .iter()
            .rposition(|row| row.kind == RowKind::Context)
            .unwrap();
        assert!(last - first >= 3, "the fixture needs rows in the middle");

        let spans = to_spans(
            &r,
            &f,
            &RenderOptions::default(),
            &Selection {
                anchor: caret(first, 2),
                focus: caret(last, 1),
            },
        );

        assert_eq!(
            spans.len(),
            last - first + 1,
            "every row in the range contributes a span"
        );

        for (offset, span) in spans.iter().enumerate().take(spans.len() - 1).skip(1) {
            assert_eq!(
                span.extent,
                SourceCharExtent::ToEnd { from: 0 },
                "row {offset} of the selection should be whole, ending included"
            );
        }
    }

    #[test]
    fn only_the_last_row_of_a_selection_stops_inside_its_line() {
        // The shape a multi-line selection actually has: every row but the last runs off the
        // end of its line and takes the line ending with it, and only the last is cut at a
        // column. This is what makes the middle of a selection paint to the pane edge rather
        // than stopping raggedly at each line's last character.
        let f = file();
        let r = rows(&f);
        let first = r
            .iter()
            .position(|row| row.kind == RowKind::Context)
            .unwrap();
        let last = r
            .iter()
            .rposition(|row| row.kind == RowKind::Context)
            .unwrap();

        let spans = to_spans(
            &r,
            &f,
            &RenderOptions::default(),
            &Selection {
                anchor: caret(first, 3),
                focus: caret(last, 1),
            },
        );

        assert_eq!(
            spans.first().unwrap().extent,
            SourceCharExtent::ToEnd { from: 3 }
        );
        assert_eq!(
            spans.last().unwrap().extent,
            SourceCharExtent::Columns(0..1)
        );
    }

    #[test]
    fn a_selection_inside_one_line_never_takes_the_ending() {
        let f = file();
        let r = rows(&f);
        let row = removed_row(&r);

        let spans = to_spans(
            &r,
            &f,
            &RenderOptions::default(),
            &Selection {
                anchor: caret(row, 4),
                focus: caret(row, 7),
            },
        );

        assert!(matches!(spans[0].extent, SourceCharExtent::Columns(_)));
    }

    #[test]
    fn a_backwards_drag_selects_the_same_range() {
        let f = file();
        let r = rows(&f);
        let opts = RenderOptions::default();
        let forwards = Selection {
            anchor: caret(1, 4),
            focus: caret(2, 9),
        };
        let backwards = Selection {
            anchor: caret(2, 9),
            focus: caret(1, 4),
        };

        assert_eq!(
            to_spans(&r, &f, &opts, &forwards),
            to_spans(&r, &f, &opts, &backwards)
        );
    }

    #[test]
    fn an_empty_selection_covers_nothing() {
        let f = file();
        let r = rows(&f);
        let at = caret(1, 4);
        assert!(to_spans(&r, &f, &RenderOptions::default(), &Selection::at(at)).is_empty());
    }
}
