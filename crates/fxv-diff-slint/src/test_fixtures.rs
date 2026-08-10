//! A parsed diff for tests that need one.
//!
//! Real `git diff` output like every other fixture, chosen for one property: the changed line
//! begins with a tab, so display columns and source character offsets genuinely differ and a
//! test that confuses the two fails rather than passing by coincidence.

// == Std
use std::fs;

// == Internal Crates
use crate::diff::layout::{Layout, RowOptions, build_inline, build_split};
use crate::diff::model::FileDiff;
use crate::diff::parse::parse_unified_diff;
use crate::diff::render::{DiffPane, render_diff};
use crate::text::RenderOptions;
use crate::view::{DisplayedRow, RowClass, RowModel};

pub fn file() -> FileDiff {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tab_line.diff");
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    parse_unified_diff(&text)
        .expect("the fixture should parse")
        .files
        .remove(0)
}

pub fn inline(file: &FileDiff) -> Layout {
    build_inline(file, &RowOptions::default())
}

pub fn split(file: &FileDiff) -> Layout {
    build_split(file, &RowOptions::default())
}

/// One pane's worth of rendered rows, which is what the selection code works on.
pub fn shown(layout: &Layout, file: &FileDiff, pane: DiffPane) -> Vec<DisplayedRow> {
    view(layout, file, pane).rows().to_vec()
}

/// One pane, which is what the highlight code works on: it resolves a line to a row through
/// the model's index rather than searching the rows itself.
pub fn view(layout: &Layout, file: &FileDiff, pane: DiffPane) -> RowModel {
    RowModel::from_rows(render_diff(layout, file, &RenderOptions::default(), pane))
}

/// The inline pane of the fixture, the usual starting point.
pub fn rows(file: &FileDiff) -> Vec<DisplayedRow> {
    shown(&inline(file), file, DiffPane::Inline)
}

/// The same pane, kept whole.
pub fn inline_view(file: &FileDiff) -> RowModel {
    view(&inline(file), file, DiffPane::Inline)
}

/// The removed line, whose source begins with a tab.
pub fn removed_row(rows: &[DisplayedRow]) -> usize {
    rows.iter()
        .position(|r| r.class == RowClass::REMOVED)
        .expect("the fixture removes a line")
}
