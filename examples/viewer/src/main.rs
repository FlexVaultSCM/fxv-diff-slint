// == Std crates
use std::cell::RefCell;
use std::env;
use std::fs;
use std::rc::Rc;

// == Internal Crates
use fxv_diff_slint::{
    build_inline, build_side_by_side, parse_unified_diff, DiffSet, FileDiff, RenderOptions,
    RowOptions, ViewRows,
};

// == External Crates
use slint::{ModelRc, SharedString, VecModel};

// Machine-generated; see the note on the library's `ui` module.
#[allow(clippy::absolute_paths)]
mod ui {
    slint::include_modules!();
}
use ui::*;

/// Diffs to browse, most taken from this repository's own history so they contain real code
/// rather than something contrived.
const SAMPLES: &[(&str, &str)] = &[
    (
        "Gaps and tabs",
        include_str!("../samples/synthetic_gaps.diff"),
    ),
    ("Slint markup", include_str!("../samples/viewer_slint.diff")),
    (
        "Rust and Slint",
        include_str!("../samples/rust_and_slint.diff"),
    ),
    (
        "Add parsing (15 files)",
        include_str!("../samples/add_parsing_commit.diff"),
    ),
    (
        "Whole crate (21 files)",
        include_str!("../samples/whole_crate.diff"),
    ),
    (
        "Adds, renames, binary",
        include_str!("../samples/mixed_operations.diff"),
    ),
];

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let diff = Rc::new(RefCell::new(DiffSet::default()));

    // A path on the command line replaces the built-in samples.
    let from_file = env::args().nth(1).map(|path| {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
        (path, text)
    });

    let names: Vec<SharedString> = match &from_file {
        Some((path, _)) => vec![path.as_str().into()],
        None => SAMPLES.iter().map(|(name, _)| (*name).into()).collect(),
    };
    window.set_sample_names(ModelRc::from(Rc::new(VecModel::from(names))));

    {
        let window = window.as_weak();
        let diff = diff.clone();
        let owned = from_file.clone();
        window.unwrap().on_sample_changed(move |index| {
            let w = window.unwrap();
            let text = match &owned {
                Some((_, text)) => text.as_str(),
                None => SAMPLES[index.max(0) as usize].1,
            };
            show_sample(&w, &diff, text);
        });
    }

    {
        let window = window.as_weak();
        let diff = diff.clone();
        window.unwrap().on_file_changed(move |index| {
            show_file(&window.unwrap(), &diff.borrow(), index.max(0) as usize);
        });
    }

    // Whitespace and layout both change what has to be built, so both go back through
    // show_file rather than restyling what is already there.
    for install in [
        MainWindow::on_whitespace_changed as fn(&MainWindow, _),
        MainWindow::on_layout_changed as fn(&MainWindow, _),
    ] {
        let window = window.as_weak();
        let diff = diff.clone();
        install(
            &window.unwrap(),
            Box::new(move || {
                let w = window.unwrap();
                let index = w.get_current_file().max(0) as usize;
                show_file(&w, &diff.borrow(), index);
            }),
        );
    }

    let first = from_file
        .as_ref()
        .map_or(SAMPLES[0].1, |(_, text)| text.as_str());
    show_sample(&window, &diff, first);

    window.run()
}

/// Parses a diff and shows its first file.
fn show_sample(window: &MainWindow, diff: &RefCell<DiffSet>, text: &str) {
    match parse_unified_diff(text) {
        Ok(parsed) => *diff.borrow_mut() = parsed,
        Err(e) => {
            window.set_status(format!("{e}").into());
            *diff.borrow_mut() = DiffSet::default();
        }
    }

    let borrowed = diff.borrow();
    let names: Vec<SharedString> = borrowed.files.iter().map(|f| describe(f).into()).collect();
    window.set_file_names(ModelRc::from(Rc::new(VecModel::from(names))));
    window.set_current_file(0);
    show_file(window, &borrowed, 0);
}

/// Builds and shows the rows for one file of the current diff.
fn show_file(window: &MainWindow, diff: &DiffSet, index: usize) {
    let Some(file) = diff.files.get(index) else {
        window.set_status("nothing to show".into());
        return;
    };

    if file.is_binary() {
        window.set_status("binary file, no content to show".into());
        return;
    }
    if file.hunks().is_empty() {
        window.set_status("no content change".into());
        return;
    }
    window.set_status(SharedString::new());

    // A different file is a different document, so it starts at the top rather than wherever
    // the last one was left.
    window.set_shared_scroll_y(0.0);
    window.set_shared_scroll_x(0.0);

    // Rebuilt rather than restyled: making whitespace visible changes the text itself, so the
    // rows have to be rendered again.
    let opts = RowOptions {
        render: RenderOptions {
            show_space_tabs: window.get_whitespace_mode() >= 1,
            show_line_endings: window.get_whitespace_mode() >= 2,
            ..Default::default()
        },
        ..Default::default()
    };
    // Only the layout on screen is built. Building both would mean rendering the whole diff
    // three times over on every file change and every toggle, for two row sets nobody sees.
    if window.get_side_by_side() {
        let split = build_side_by_side(file, &opts);
        // One column count for both panes; see SideBySideRows::longest_line_columns.
        window.set_split_columns(split.longest_line_columns() as i32);
        window.set_left_rows(ViewRows::from(&split.left).rows);
        window.set_right_rows(ViewRows::from(&split.right).rows);
    } else {
        let inline = ViewRows::from(&build_inline(file, &opts));
        window.set_inline_columns(inline.longest_line_columns);
        window.set_inline_rows(inline.rows);
    }
}

/// A label for the file picker, noting anything that happened beyond an edit.
fn describe(file: &FileDiff) -> String {
    use fxv_diff_slint::FileChange;

    let note = match file.change {
        FileChange::Added => "  [added]",
        FileChange::Removed => "  [removed]",
        FileChange::Renamed => "  [renamed]",
        FileChange::Copied => "  [copied]",
        FileChange::Modified if file.left_mode != file.right_mode => "  [mode]",
        FileChange::Modified => "",
    };
    let binary = if file.is_binary() { "  [binary]" } else { "" };
    format!("{}{note}{binary}", file.display_path())
}
