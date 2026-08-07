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
use std::collections::HashMap;
use std::ops::Range;

// == Internal Crates
use crate::model::FileDiff;
use crate::rows::Row;
use crate::span::{LineSpan, Side, SourceCharExtent};
use crate::text::{display_column_of, map_span, RenderOptions};

/// Why a range is drawn, which is what picks the colour.
///
/// WIP: becomes an open set of channels, each with its own style configured on the view, so a
/// host can add its own kinds without this enum knowing about them. The two here are what the
/// current callers need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// What the user is selecting now.
    Selection,
    /// Supplied from outside: a stored selection, or something a host is pointing at.
    Marked,
}

/// How much of one row a highlight covers, in the columns the grid is drawn on.
///
/// The same two cases as `SourceCharExtent`, converted through `map_span`. Kept as a separate
/// type rather than shared: these are the coordinates the view works in, and they move when
/// the tab width or the whitespace options change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderColumnExtent {
    /// Columns `range` of the row.
    Columns(Range<u32>),
    /// From `from` to the edge of the pane, which is how a covered line ending is shown.
    ToEnd { from: u32 },
}

/// A range to paint behind one row's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    pub extent: RenderColumnExtent,
    pub kind: HighlightKind,
}

/// Where to draw a set of spans, as a highlight against the row that carries each line.
///
/// Spans naming a line the view is not currently showing are dropped rather than reported. A
/// host restoring a stored selection may well hand back lines that are inside a gap now, and
/// that is ordinary rather than an error. It also means the same stored set can be given to
/// both panes of a split view, each drawing only what belongs to it.
///
/// WIP: the line index is rebuilt on every call, so this costs a walk of every row whatever
/// the spans ask for. That is the wrong frequency for the case it was written for, where a
/// host's marks stay put while something else changes. It moves to an index built once per
/// row model and kept.
pub fn to_highlights(
    rows: &[Row],
    file: &FileDiff,
    opts: &RenderOptions,
    context_side: Side,
    spans: &[LineSpan],
    kind: HighlightKind,
) -> Vec<(usize, Highlight)> {
    let mut by_line: HashMap<(Side, u32), usize> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some((side, line)) = row.file_line(context_side) {
            by_line.entry((side, line)).or_insert(index);
        }
    }

    let mut out = Vec::new();
    for span in spans {
        let Some(&index) = by_line.get(&(span.side, span.line)) else {
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
                RenderColumnExtent::Columns(columns.start as u32..columns.end as u32)
            }
            // Where it ends is not a column at all, so only the start is converted.
            SourceCharExtent::ToEnd { from } => RenderColumnExtent::ToEnd {
                from: display_column_of(text, *from as usize, opts) as u32,
            },
        };

        out.push((index, Highlight { extent, kind }));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{build_inline, RowOptions};
    use crate::selection::{to_spans, Caret, Selection};
    use crate::test_fixtures::{file, removed_row, rows};

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

        let stored = to_spans(&r, &f, &opts, Side::Right, &selection);
        let drawn_live = to_highlights(
            &r,
            &f,
            &opts,
            Side::Right,
            &stored,
            HighlightKind::Selection,
        );
        let drawn_again = to_highlights(
            &r,
            &f,
            &opts,
            Side::Right,
            &stored,
            HighlightKind::Selection,
        );

        assert!(
            !drawn_live.is_empty(),
            "the fixture should select something"
        );
        assert_eq!(drawn_live, drawn_again);
    }

    #[test]
    fn a_stored_selection_still_lands_after_the_rows_are_rebuilt() {
        // Row indices are not part of the description, so rebuilding the rows with different
        // options must not move a stored span.
        let f = file();
        let plain = RenderOptions::default();
        let visible = RenderOptions {
            show_space_tabs: true,
            ..RenderOptions::default()
        };

        let before = build_inline(&f, &RowOptions::default()).rows;
        let spans = to_spans(
            &before,
            &f,
            &plain,
            Side::Right,
            &Selection {
                anchor: caret(1, 4),
                focus: caret(1, 7),
            },
        );

        let after = build_inline(
            &f,
            &RowOptions {
                render: visible.clone(),
                ..RowOptions::default()
            },
        )
        .rows;
        let drawn = to_highlights(
            &after,
            &f,
            &visible,
            Side::Right,
            &spans,
            HighlightKind::Marked,
        );

        assert_eq!(
            drawn.len(),
            1,
            "the span still names a line that is on screen"
        );
        assert_eq!(drawn[0].0, 1, "and the same row");
    }

    #[test]
    fn a_span_naming_a_line_that_is_not_shown_is_dropped() {
        let f = file();
        let r = rows(&f);
        let spans = vec![LineSpan {
            side: Side::Right,
            line: 9999,
            extent: SourceCharExtent::Columns(0..3),
        }];

        let drawn = to_highlights(
            &r,
            &f,
            &RenderOptions::default(),
            Side::Right,
            &spans,
            HighlightKind::Marked,
        );
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
            side: Side::Left,
            line: 11,
            extent: SourceCharExtent::ToEnd { from: 1 },
        }];
        let drawn = to_highlights(&r, &f, &opts, Side::Right, &spans, HighlightKind::Marked);

        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].0, row);
        assert_eq!(
            drawn[0].1.extent,
            RenderColumnExtent::ToEnd { from: 4 },
            "source character 1 sits at display column 4, past the tab"
        );
    }
}
