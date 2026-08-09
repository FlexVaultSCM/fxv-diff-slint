//! Turning a laid-out diff into rows a pane can draw.
//!
//! The one place that reads diff vocabulary and writes display vocabulary. A pane knows nothing
//! about additions, removals or hunks; this is where those become a row kind, a gutter, and a
//! piece of rendered text.
//!
//! One layout serves both panes of a split view. Each reads its own side of every row, which is
//! why a filler is simply the side that was absent rather than a row pretending to be one.

// == Internal Crates
use super::layout::{Layout, Line, Row};
use super::model::{FileDiff, LineKind};
use crate::span::Side;
use crate::text::{render_line, RenderOptions};
use crate::view::{DisplayedRow, RowClass, GUTTER_COLUMNS};

/// Which part of a layout a pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// One column showing both files, with two numbers against an unchanged line.
    Inline,
    /// The left file only, as one side of a split view.
    Left,
    /// The right file only.
    Right,
}

/// Renders one pane's worth of a layout.
///
/// The result goes to `RowModel::from_rows`, the same door a plain file listing comes through.
pub fn render_diff(
    layout: &Layout,
    file: &FileDiff,
    opts: &RenderOptions,
    pane: Pane,
) -> Vec<DisplayedRow> {
    layout
        .rows
        .iter()
        .map(|row| display(row, file, opts, pane))
        .collect()
}

/// Renders one entry as this pane sees it.
fn display(entry: &Row, file: &FileDiff, opts: &RenderOptions, pane: Pane) -> DisplayedRow {
    match entry {
        // The file itself rather than a line in it, so it runs across the gutter and numbers
        // nothing.
        Row::Header => DisplayedRow {
            text: file.display_path().into(),
            columns: file.display_path().chars().count() as u32,
            full_width: true,
            ..DisplayedRow::blank(RowClass::HEADER)
        },

        Row::Gap {
            left_start,
            right_start,
            hidden,
            state,
            pending,
            heading,
            reason,
        } => {
            // A failure explains itself; otherwise the heading names what follows.
            let note = reason.as_deref().or(heading.as_deref()).unwrap_or_default();
            DisplayedRow {
                hidden_count: *hidden,
                gap_state: *state,
                note: note.into(),
                gap_start: (*left_start, *right_start),
                pending: *pending,
                // The band drawn over a gap takes the gutter with it, and a gap names no line.
                full_width: true,
                ..DisplayedRow::blank(RowClass::GAP)
            }
        }

        Row::Lines(diff_row) => {
            let shown = match pane {
                Pane::Left => diff_row.left,
                Pane::Right => diff_row.right,
                // Inline draws one line per entry, so whichever side is present is the one to
                // draw. An unchanged line is on both and either gives the same text.
                Pane::Inline => diff_row.left.or(diff_row.right),
            };

            let Some(row) = shown else {
                return DisplayedRow::blank(RowClass::FILLER);
            };
            let Some(line) = file.line(row.source) else {
                return DisplayedRow::blank(RowClass::FILLER);
            };

            let (text, columns) = render_line(&line.text, line.line_ending, opts);
            let side = match line.kind {
                LineKind::Removed => Side::Left,
                LineKind::Added => Side::Right,
                // An unchanged line is in both files, so which one names it is decided by the
                // pane. An inline view means the right, being the file as it stands now.
                LineKind::Context => match pane {
                    Pane::Left => Side::Left,
                    Pane::Inline | Pane::Right => Side::Right,
                },
            };

            DisplayedRow {
                class: match line.kind {
                    LineKind::Context => RowClass::CONTEXT,
                    LineKind::Added => RowClass::ADDED,
                    LineKind::Removed => RowClass::REMOVED,
                },
                numbered: true,
                numbers: gutter(diff_row.left, diff_row.right, pane),
                id: Some((side, row.line)),
                source: Some(row.source),
                text: text.as_str().into(),
                columns: columns as u32,
                selectable: true,
                ..DisplayedRow::blank(RowClass::CONTEXT)
            }
        }
    }
}

/// The numbers this pane's gutter shows for an entry.
fn gutter(left: Option<Line>, right: Option<Line>, pane: Pane) -> [Option<u32>; GUTTER_COLUMNS] {
    match pane {
        Pane::Inline => [left.map(|r| r.line), right.map(|r| r.line)],
        Pane::Left => [left.map(|r| r.line), None],
        Pane::Right => [right.map(|r| r.line), None],
    }
}
