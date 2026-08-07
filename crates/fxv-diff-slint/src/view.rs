//! Handing rows to the widget.
//!
//! The row model uses `Option<u32>` for line numbers, because a line genuinely has no number
//! on the side it does not exist on. The Slint struct cannot: the language has no optional
//! type, so absence is encoded as zero, which is safe because line numbers are 1-based.
//! This module is where that translation happens, and the only place that knows about it.
//!
//! The text needs no translation: rows already hold it in the form the widget takes, so
//! handing a row over shares its text rather than copying it.
//!
//! The model is kept rather than rebuilt. Highlights change while a pointer is moving, and
//! replacing the whole model on every move would rebuild every row to change two of them.

// == Std crates
use std::mem;
use std::rc::Rc;

// == External Crates
use slint::{Model, ModelRc, VecModel};

// == Internal Crates
use crate::highlight::{Highlight, HighlightKind, RenderColumnExtent};
use crate::rows::{GapState, Row, RowKind, Rows};
use crate::ui;

/// The rows a view is showing, in both the form this crate reasons about and the form the
/// widget draws, kept together so highlights can change without either being rebuilt.
pub struct RowModel {
    rows: Vec<Row>,
    model: Rc<VecModel<ui::DiffRow>>,
    longest_line_columns: i32,
    /// What each row that carries anything is currently drawing, in row order.
    ///
    /// Kept so a change can write back only the rows whose highlights actually differ. Handing
    /// Slint a row it already holds counts as a change to the model, so the list invalidates
    /// and redraws that row for nothing.
    drawn: Vec<(usize, Vec<Highlight>)>,
}

impl RowModel {
    pub fn new(rows: Rows) -> Self {
        let converted: Vec<ui::DiffRow> = rows.rows.iter().map(|r| r.into()).collect();
        RowModel {
            rows: rows.rows,
            model: Rc::new(VecModel::from(converted)),
            longest_line_columns: rows.longest_line_columns as i32,
            drawn: Vec::new(),
        }
    }

    /// What the widget binds to.
    pub fn model(&self) -> ModelRc<ui::DiffRow> {
        ModelRc::from(self.model.clone())
    }

    /// The rows themselves, for the selection arithmetic, which works in this crate's terms.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Columns the longest line occupies, which is how a view sizes its horizontal scrolling.
    pub fn longest_line_columns(&self) -> i32 {
        self.longest_line_columns
    }

    /// Replaces every highlight on screen.
    ///
    /// Rows whose highlights come out the same are left alone rather than rewritten, which is
    /// what keeps a change costing the rows it touched rather than every row it named.
    ///
    /// Returns how many rows were written, which is the measure of what this cost.
    ///
    /// WIP: this takes one flat list, so a caller with two sources has to merge them and
    /// resend both whenever either changes. It becomes one call per channel next, at which
    /// point a change to one channel stops touching the other's rows at all.
    pub fn set_highlights(&mut self, highlights: &[(usize, Highlight)]) -> usize {
        let wanted = group_by_row(highlights, self.rows.len());
        // Taken out so the loops below read a local rather than borrowing `self`, which is
        // what lets `write` take `&mut self` and keep an ordinary counter.
        let previous = mem::take(&mut self.drawn);
        let mut written = 0;

        // Rows that had something and no longer do.
        for (index, _) in &previous {
            if wanted.binary_search_by_key(index, |(i, _)| *i).is_err() {
                written += usize::from(self.write(*index, &[]));
            }
        }

        for (index, list) in &wanted {
            let unchanged = previous
                .binary_search_by_key(index, |(i, _)| *i)
                .is_ok_and(|at| previous[at].1 == *list);
            if !unchanged {
                written += usize::from(self.write(*index, list));
            }
        }

        self.drawn = wanted;
        written
    }

    /// Hands one row back to Slint. Reports whether it did, since a row that has gone missing
    /// is not a write.
    fn write(&mut self, index: usize, highlights: &[Highlight]) -> bool {
        let Some(mut row) = self.model.row_data(index) else {
            return false;
        };
        row.highlights = to_slint_highlights(highlights);
        self.model.set_row_data(index, row);
        true
    }
}

/// Gathers highlights per row, in row order, dropping any that name a row that is not there.
///
/// Sorted rather than grouped in place so both this and the previous state can be walked by
/// binary search, instead of scanning one for every entry of the other.
fn group_by_row(highlights: &[(usize, Highlight)], rows: usize) -> Vec<(usize, Vec<Highlight>)> {
    let mut ordered: Vec<&(usize, Highlight)> =
        highlights.iter().filter(|(i, _)| *i < rows).collect();
    ordered.sort_by_key(|(i, _)| *i);

    let mut grouped: Vec<(usize, Vec<Highlight>)> = Vec::new();
    for (index, highlight) in ordered {
        match grouped.last_mut() {
            Some((at, list)) if at == index => list.push(highlight.clone()),
            _ => grouped.push((*index, vec![highlight.clone()])),
        }
    }
    grouped
}

/// Packs a row's highlights into the form the widget binds to.
///
/// A row with none costs nothing: `ModelRc::default()` holds no model at all. A row with any
/// costs two allocations, the vector and the model wrapping it.
///
/// WIP: those two could be one, and the row need not be rewritten at all. `Model::as_any`
/// downcasts a `ModelRc` back to its `VecModel`, and `VecModel::set_vec` replaces the contents
/// through a shared reference, so a row could be given its model once and have the contents
/// swapped afterwards. The gain is not the allocation: changing a row's highlights would stop
/// counting as a change to the row itself, so the list would re-evaluate the highlights alone
/// instead of the row. Left until the write path is restructured for channels, because it
/// changes what "a row was written" means and the tests here measure exactly that.
fn to_slint_highlights(highlights: &[Highlight]) -> ModelRc<ui::DiffHighlight> {
    if highlights.is_empty() {
        return ModelRc::default();
    }
    let converted: Vec<ui::DiffHighlight> = highlights
        .iter()
        .map(|h| {
            // Slint has no enum carrying a payload, so the two cases flatten into a flag.
            // `end` is meaningless when `to_end` is set; the row runs to its own edge.
            let (start, end, to_end) = match &h.extent {
                RenderColumnExtent::Columns(columns) => {
                    (columns.start as i32, columns.end as i32, false)
                }
                RenderColumnExtent::ToEnd { from } => (*from as i32, 0, true),
            };
            ui::DiffHighlight {
                start,
                end,
                to_end,
                kind: h.kind.into(),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(converted)))
}

impl From<HighlightKind> for ui::DiffHighlightKind {
    fn from(kind: HighlightKind) -> Self {
        match kind {
            HighlightKind::Selection => ui::DiffHighlightKind::Selection,
            HighlightKind::Marked => ui::DiffHighlightKind::Marked,
        }
    }
}

impl From<&Row> for ui::DiffRow {
    fn from(row: &Row) -> Self {
        ui::DiffRow {
            kind: row.kind.into(),
            // Zero means "no line on this side". Line numbers are 1-based, so it cannot collide
            // with a real one.
            left_line: row.left_line.unwrap_or(0) as i32,
            right_line: row.right_line.unwrap_or(0) as i32,
            text: row.text.clone(),
            hidden_count: row.hidden_count as i32,
            gap_state: row.gap_state.into(),
            highlights: ModelRc::default(),
        }
    }
}

impl From<GapState> for ui::DiffGapState {
    fn from(state: GapState) -> Self {
        match state {
            GapState::Hidden => ui::DiffGapState::Hidden,
            GapState::Waiting => ui::DiffGapState::Waiting,
            GapState::Failed => ui::DiffGapState::Failed,
        }
    }
}

impl From<RowKind> for ui::DiffRowKind {
    fn from(kind: RowKind) -> Self {
        match kind {
            RowKind::Header => ui::DiffRowKind::Header,
            RowKind::Context => ui::DiffRowKind::Context,
            RowKind::Added => ui::DiffRowKind::Added,
            RowKind::Removed => ui::DiffRowKind::Removed,
            RowKind::Gap => ui::DiffRowKind::Gap,
            RowKind::Filler => ui::DiffRowKind::Filler,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    fn row(kind: RowKind, left: Option<u32>, right: Option<u32>, text: &str) -> Row {
        Row {
            kind,
            left_line: left,
            right_line: right,
            columns: text.chars().count() as u32,
            text: text.into(),
            hidden_count: 0,
            gap_state: GapState::Hidden,
            source: None,
        }
    }

    #[test]
    fn a_missing_line_number_becomes_zero() {
        let rows = Rows {
            rows: vec![row(RowKind::Added, None, Some(7), "new")],
            longest_line_columns: 3,
        };
        let view = RowModel::new(rows);
        let converted = view.model().row_data(0).unwrap();
        assert_eq!(converted.left_line, 0, "absent on the left");
        assert_eq!(converted.right_line, 7);
    }

    #[test]
    fn the_measurement_travels_with_the_rows() {
        let rows = Rows {
            rows: vec![row(RowKind::Context, Some(1), Some(1), "a line")],
            longest_line_columns: 6,
        };
        assert_eq!(RowModel::new(rows).longest_line_columns(), 6);
    }

    fn mark(row: usize, columns: Range<u32>) -> (usize, Highlight) {
        (
            row,
            Highlight {
                extent: RenderColumnExtent::Columns(columns),
                kind: HighlightKind::Marked,
            },
        )
    }

    fn five_rows() -> Rows {
        Rows {
            rows: (0..5)
                .map(|n| row(RowKind::Context, Some(n), Some(n), "some text"))
                .collect(),
            longest_line_columns: 9,
        }
    }

    #[test]
    fn a_row_whose_highlights_did_not_change_is_not_written_again() {
        // Handing Slint a row it already holds counts as a change, and the list redraws it.
        let mut view = RowModel::new(five_rows());
        let same = vec![mark(1, 0..3), mark(3, 1..2)];

        assert_eq!(
            view.set_highlights(&same),
            2,
            "both highlighted rows written once"
        );
        assert_eq!(view.set_highlights(&same), 0, "and not written again");
    }

    #[test]
    fn only_the_rows_that_changed_are_written() {
        let mut view = RowModel::new(five_rows());
        view.set_highlights(&[mark(1, 0..3), mark(3, 1..2)]);

        // Row 1 keeps exactly what it had; row 3's range moves.
        let written = view.set_highlights(&[mark(1, 0..3), mark(3, 4..6)]);

        assert_eq!(written, 1, "only the row that moved");
    }

    #[test]
    fn a_row_that_loses_its_highlights_is_cleared() {
        let mut view = RowModel::new(five_rows());
        view.set_highlights(&[mark(2, 0..3)]);

        assert_eq!(view.set_highlights(&[]), 1, "cleared, once");
        let drawn = view.model().row_data(2).unwrap();
        assert_eq!(drawn.highlights.row_count(), 0);
    }

    #[test]
    fn every_kind_has_a_counterpart() {
        for kind in [
            RowKind::Header,
            RowKind::Context,
            RowKind::Added,
            RowKind::Removed,
            RowKind::Gap,
            RowKind::Filler,
        ] {
            let rows = Rows {
                rows: vec![row(kind, None, None, "")],
                longest_line_columns: 0,
            };
            // Converting must not panic, and must preserve the row.
            assert_eq!(RowModel::new(rows).model().row_count(), 1);
        }
    }
}
