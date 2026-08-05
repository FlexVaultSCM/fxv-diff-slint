// == Std crates
use std::rc::Rc;

// == Internal Crates
// The row type comes from the library crate rather than from this crate's generated code:
// there is one definition, shared across the crate boundary.
use fxv_diff_slint::ui::{DiffRow, DiffRowKind};

// == External Crates
use slint::VecModel;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;

    let rows = sample_rows();
    // Computed here only because the row-model builder that will own this does not exist yet.
    // The view pans no further than this, so it has to reflect the content.
    let longest = rows
        .iter()
        .map(|r| r.text.chars().count())
        .max()
        .unwrap_or(0);

    window.set_longest_line_columns(longest as i32);
    window.set_rows(Rc::new(VecModel::from(rows)).into());

    window.run()
}

/// Hand-written sample content.
fn sample_rows() -> Vec<DiffRow> {
    let row = |kind, left: i32, right: i32, text: &str| DiffRow {
        kind,
        left_line: left,
        right_line: right,
        text: text.into(),
        hidden_count: 0,
    };

    vec![
        DiffRow {
            kind: DiffRowKind::Gap,
            left_line: 0,
            right_line: 0,
            text: "@@ 40 lines hidden @@".into(),
            hidden_count: 40,
        },
        row(DiffRowKind::Context, 41, 41, "impl Store {"),
        row(DiffRowKind::Context, 42, 42, "    pub fn get(&self, key: &Key) -> Option<&Value> {"),
        row(DiffRowKind::Removed, 43, 0, "        self.map.get(key)"),
        row(DiffRowKind::Added, 0, 43, "        self.map.get(key).filter(|v| !v.is_expired())"),
        row(DiffRowKind::Context, 44, 44, "    }"),
        row(DiffRowKind::Context, 45, 45, ""),
        row(
            DiffRowKind::Removed,
            46,
            0,
            "    pub fn insert(&mut self, key: Key, value: Value) { self.map.insert(key, value); }",
        ),
        row(
            DiffRowKind::Added,
            0,
            46,
            "    pub fn insert(&mut self, key: Key, value: Value) -> Option<Value> { self.map.insert(key, value) }",
        ),
        row(DiffRowKind::Context, 47, 47, "}"),
        DiffRow {
            kind: DiffRowKind::Gap,
            left_line: 0,
            right_line: 0,
            text: "@@ 112 lines hidden @@".into(),
            hidden_count: 112,
        },
    ]
}
