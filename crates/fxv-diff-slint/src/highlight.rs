//! Turning spans into something to paint.
//!
//! The last stage of the pipeline that starts with a gesture: a `Selection` resolves to
//! `LineSpan`s, which name lines in a file, and those resolve here into ranges against the
//! rows currently on screen.
//!
//! Everything here is transient and view-shaped. A value of these types is measured in display
//! columns and row indices, both of which move when a gap opens or the whitespace options
//! change, so none of it is meaningful once the frame that produced it is gone. Storing one is
//! a mistake; store a `LineSpan`.
//!
//! Nothing here is specific to selection. A selection is one thing that produces ranges to
//! paint, and a host marking a line is another.

// == Std

// == Internal Crates
use crate::diff::model::FileDiff;
use crate::span::{LineSpan, SourceCharExtent};
use crate::text::{RenderOptions, display_column_of, map_span};
use crate::view::{DisplayColumnExtent, RowModel};

/// Where to draw a set of spans, as a range against the row that carries each line.
///
/// The result goes to `RowModel::set_channel`, which decides how it is painted. What the spans
/// mean is the caller's business; this only says where they land.
///
/// Spans naming a line the view is not currently showing are dropped rather than reported. A
/// host restoring a stored selection may well hand back lines that are inside a gap now, and
/// that is ordinary rather than an error. It also means the same stored set can be given to
/// both panes of a split view, each drawing only what belongs to it.
pub fn to_highlights(
    view: &RowModel,
    file: &FileDiff,
    opts: &RenderOptions,
    spans: &[LineSpan],
) -> Vec<(usize, DisplayColumnExtent)> {
    let rows = view.rows();
    let mut out = Vec::new();
    for span in spans {
        let Some(index) = view.row_of(span.document, span.line) else {
            continue;
        };
        let Some(source) = rows[index].source else {
            continue;
        };
        let Some(text) = file.line(source).map(|l| l.text.as_str()) else {
            continue;
        };

        let extent = match &span.extent {
            SourceCharExtent::Columns(chars) => {
                let columns = map_span(text, chars.start as usize..chars.end as usize, opts);
                if columns.start >= columns.end {
                    continue;
                }
                DisplayColumnExtent::Columns(columns.start as u32..columns.end as u32)
            }
            // Where it ends is not a column at all, so only the start is converted.
            SourceCharExtent::ToEnd { from } => DisplayColumnExtent::ToEnd {
                from: display_column_of(text, *from as usize, opts) as u32,
            },
        };
        out.push((index, extent));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::render::{DiffPane, render_diff};
    use crate::selection::{Caret, Selection, to_spans};
    use crate::span::Document;
    use crate::test_fixtures::{file, inline, inline_view, removed_row, rows};
    use crate::view::RowModel;

    fn caret(row: usize, column: u32) -> Caret {
        Caret { row, column }
    }

    #[test]
    fn a_selection_that_survives_a_round_trip_draws_in_the_same_place() {
        // The property a host depends on: report a selection, store it, hand it back, and get
        // the same rectangles. Everything about the description has to be independent of the
        // view for this to hold.
        let f = file();
        let r = rows(&f);
        let opts = RenderOptions::default();
        let selection = Selection {
            anchor: caret(1, 4),
            focus: caret(2, 9),
        };

        let stored = to_spans(&r, &f, &opts, &selection);
        let v = inline_view(&f);
        let drawn_live = to_highlights(&v, &f, &opts, &stored);
        let drawn_again = to_highlights(&v, &f, &opts, &stored);

        assert!(
            !drawn_live.is_empty(),
            "the fixture should select something"
        );
        assert_eq!(drawn_live, drawn_again);
    }

    #[test]
    fn a_stored_span_survives_the_rows_being_drawn_differently() {
        // The layout is the same either way: showing whitespace changes how a line reads, not
        // where it sits. So the same entries are rendered twice, and a span reported from one
        // rendering still lands on the same row in the other, at whatever columns that
        // rendering puts it.
        let f = file();
        let layout = inline(&f);
        let plain = RenderOptions::default();
        let visible = RenderOptions {
            show_space_tabs: true,
            ..RenderOptions::default()
        };

        let before = RowModel::from_rows(render_diff(&layout, &f, &plain, DiffPane::Inline))
            .rows()
            .to_vec();
        let row = removed_row(&before);
        let spans = to_spans(
            &before,
            &f,
            &plain,
            &Selection {
                anchor: caret(row, 4),
                focus: caret(row, 7),
            },
        );

        let after = RowModel::from_rows(render_diff(&layout, &f, &visible, DiffPane::Inline));
        let drawn = to_highlights(&after, &f, &visible, &spans);

        assert_eq!(
            drawn.len(),
            1,
            "the span still names a line that is on screen"
        );
        assert_eq!(drawn[0].0, row, "and the same row");
    }

    #[test]
    fn a_span_naming_a_line_that_is_not_shown_is_dropped() {
        let f = file();
        let spans = vec![LineSpan {
            document: Document::AFTER,
            line: 9999,
            extent: SourceCharExtent::Columns(0..3),
        }];

        let drawn = to_highlights(&inline_view(&f), &f, &RenderOptions::default(), &spans);
        assert!(drawn.is_empty(), "a line inside a gap is not an error");
    }

    #[test]
    fn a_covered_line_ending_converts_to_a_run_with_no_end() {
        // A tab still shifts where the run starts, so the conversion is a real one rather
        // than a pass-through of the source offset.
        let f = file();
        let r = rows(&f);
        let row = removed_row(&r);
        let opts = RenderOptions::default();

        let spans = vec![LineSpan {
            document: Document::BEFORE,
            line: 11,
            extent: SourceCharExtent::ToEnd { from: 1 },
        }];
        let drawn = to_highlights(&inline_view(&f), &f, &opts, &spans);

        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].0, row);
        assert_eq!(
            drawn[0].1,
            DisplayColumnExtent::ToEnd { from: 4 },
            "source character 1 sits at display column 4, past the tab"
        );
    }
}
