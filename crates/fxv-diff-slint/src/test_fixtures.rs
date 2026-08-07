//! A parsed diff for tests that need one.
//!
//! Real `git diff` output like every other fixture, chosen for one property: the changed line
//! begins with a tab, so display columns and source character offsets genuinely differ and a
//! test that confuses the two fails rather than passing by coincidence.

// == Std
use std::fs;

// == Internal Crates
use crate::model::FileDiff;
use crate::parse::parse_unified_diff;
use crate::rows::{build_inline, Row, RowKind, RowOptions};

pub fn file() -> FileDiff {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tab_line.diff");
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    parse_unified_diff(&text)
        .expect("the fixture should parse")
        .files
        .remove(0)
}

pub fn rows(file: &FileDiff) -> Vec<Row> {
    build_inline(file, &RowOptions::default()).rows
}

/// The removed line, whose source begins with a tab.
pub fn removed_row(rows: &[Row]) -> usize {
    rows.iter()
        .position(|r| r.kind == RowKind::Removed)
        .expect("the fixture removes a line")
}
