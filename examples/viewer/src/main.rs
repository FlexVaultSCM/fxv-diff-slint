// == Std crates
use std::env;
use std::fs;

// == Internal Crates
use fxv_diff_slint::{build_inline, build_side_by_side, parse_unified_diff, RowOptions, ViewRows};

slint::include_modules!();

/// A diff to show when none is given on the command line.
const SAMPLE: &str = include_str!("../sample.diff");

fn main() -> Result<(), slint::PlatformError> {
    let text = match env::args().nth(1) {
        Some(path) => fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}")),
        None => SAMPLE.to_owned(),
    };

    let diff = match parse_unified_diff(&text) {
        Ok(diff) => diff,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let Some(file) = diff.files.first() else {
        eprintln!("the diff is empty");
        std::process::exit(1);
    };

    let opts = RowOptions::default();
    let inline = ViewRows::from(&build_inline(file, &opts));
    let split = build_side_by_side(file, &opts);
    let left = ViewRows::from(&split.left);
    let right = ViewRows::from(&split.right);

    let window = MainWindow::new()?;
    window.set_file_path(file.display_path().into());

    window.set_inline_columns(inline.longest_line_columns);
    window.set_inline_rows(inline.rows);

    window.set_left_columns(left.longest_line_columns);
    window.set_left_rows(left.rows);
    window.set_right_columns(right.longest_line_columns);
    window.set_right_rows(right.rows);

    window.run()
}
