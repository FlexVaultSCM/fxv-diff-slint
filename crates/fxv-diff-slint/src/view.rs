//! Handing rows to the widget.
//!
//! The row model uses `Option<u32>` for line numbers, because a line genuinely has no number
//! on the side it does not exist on. The Slint struct cannot: the language has no optional
//! type, so absence is encoded as zero, which is safe because line numbers are 1-based.
//! This module is where that translation happens, and the only place that knows about it.

// == Std crates
use std::rc::Rc;

// == External Crates
use slint::{ModelRc, VecModel};

// == Internal Crates
use crate::rows::{Row, RowKind, Rows};
use crate::ui;

/// Rows in the form the widget takes, together with the measurement it needs to size its
/// horizontal scrolling.
pub struct ViewRows {
    pub rows: ModelRc<ui::DiffRow>,
    pub longest_line_columns: i32,
}

impl From<&Rows> for ViewRows {
    fn from(rows: &Rows) -> Self {
        let converted: Vec<ui::DiffRow> = rows.rows.iter().map(to_slint_row).collect();
        ViewRows {
            rows: ModelRc::from(Rc::new(VecModel::from(converted))),
            longest_line_columns: rows.longest_line_columns as i32,
        }
    }
}

fn to_slint_row(row: &Row) -> ui::DiffRow {
    ui::DiffRow {
        kind: to_slint_kind(row.kind),
        // Zero means "no line on this side". Line numbers are 1-based, so it cannot collide
        // with a real one.
        left_line: row.left_line.unwrap_or(0) as i32,
        right_line: row.right_line.unwrap_or(0) as i32,
        text: row.text.as_str().into(),
        hidden_count: row.hidden_count as i32,
    }
}

fn to_slint_kind(kind: RowKind) -> ui::DiffRowKind {
    match kind {
        RowKind::Header => ui::DiffRowKind::Header,
        RowKind::Context => ui::DiffRowKind::Context,
        RowKind::Added => ui::DiffRowKind::Added,
        RowKind::Removed => ui::DiffRowKind::Removed,
        RowKind::Gap => ui::DiffRowKind::Gap,
        RowKind::Filler => ui::DiffRowKind::Filler,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    fn row(kind: RowKind, left: Option<u32>, right: Option<u32>, text: &str) -> Row {
        Row {
            kind,
            left_line: left,
            right_line: right,
            columns: text.chars().count() as u32,
            text: text.to_owned(),
            hidden_count: 0,
            source: None,
        }
    }

    #[test]
    fn a_missing_line_number_becomes_zero() {
        let rows = Rows {
            rows: vec![row(RowKind::Added, None, Some(7), "new")],
            longest_line_columns: 3,
        };
        let view = ViewRows::from(&rows);
        let converted = view.rows.row_data(0).unwrap();
        assert_eq!(converted.left_line, 0, "absent on the left");
        assert_eq!(converted.right_line, 7);
    }

    #[test]
    fn the_measurement_travels_with_the_rows() {
        let rows = Rows {
            rows: vec![row(RowKind::Context, Some(1), Some(1), "a line")],
            longest_line_columns: 6,
        };
        assert_eq!(ViewRows::from(&rows).longest_line_columns, 6);
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
            assert_eq!(ViewRows::from(&rows).rows.row_count(), 1);
        }
    }
}
