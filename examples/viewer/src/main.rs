// == Std crates
use std::cell::RefCell;
use std::env;
use std::fs;
use std::rc::Rc;
use std::time;

// == Internal Crates
use fxv_diff_slint::{
    build_inline, build_split, parse_unified_diff, DiffSet, FileDiff, Pane, RenderOptions,
    RowModel, RowOptions,
};

// == External Crates
use slint::{ModelRc, SharedString, Timer, VecModel};

// Machine-generated; see the note on the library's `ui` module.
//
// dead_code because a consumer re-parses the library's .slint sources and re-embeds the images
// they reference, while the globals holding them come from the library crate. The duplicates
// are never read, and nothing here can stop them being emitted.
#[allow(clippy::absolute_paths, dead_code)]
mod ui {
    slint::include_modules!();
}
use ui::*;

/// Diffs to browse, most taken from this repository's own history so they contain real code
/// rather than something contrived.
/// The right-hand file a sample's gaps can be opened from, where we have it. Without one the
/// gaps are still shown, but there is nothing to fill them with.
const SAMPLE_SOURCES: &[(&str, &str)] = &[(
    "Gaps and tabs",
    include_str!("../samples/synthetic_gaps.after.txt"),
)];

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

    {
        let window = window.as_weak();
        let diff = diff.clone();
        // One timer per request, kept alive past the closure that started it, since a dropped
        // Timer never fires. Sharing one would cancel whichever gap was already waiting and
        // leave it saying "loading" for good.
        let pending: Rc<RefCell<Vec<Timer>>> = Rc::new(RefCell::new(Vec::new()));
        // The sample picker lists the file's one diff when there is one, so its index no longer
        // selects among the built-in samples.
        let from_file = from_file.is_some();
        window.unwrap().on_gap_expand_requested(move |request| {
            let w = window.unwrap();
            let index = w.get_current_file().max(0) as usize;
            let sample = (!from_file).then(|| w.get_current_sample().max(0) as usize);
            let start = request.right_start.max(1) as u32;
            let count = request.count.max(0) as u32;

            if let Some(file) = diff.borrow_mut().files.get_mut(index) {
                // A new attempt supersedes whatever the last one said.
                file.clear_failed_fetches();
                file.fetch_started(start, count);
            }
            rebuild_rows(&w, &diff.borrow(), index);

            // A real host reads another revision of the file, over a network as often as not.
            // Answering on a timer instead of straight away is what makes the waiting and
            // failure states reachable here at all.
            let window = w.as_weak();
            let diff = diff.clone();
            let left = request.left_start.max(1) as u32;
            let timer = Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                time::Duration::from_millis(600),
                move || {
                    let w = window.unwrap();
                    if let Some(file) = diff.borrow_mut().files.get_mut(index) {
                        match fetch(sample, start, count) {
                            // The request names a run on each side, and the lines are the same
                            // either way, so both numbers go in.
                            Ok(lines) => file.expand(left, start, lines),
                            Err(why) => file.fetch_failed(start, count, why),
                        }
                    }
                    rebuild_rows(&w, &diff.borrow(), index);
                },
            );
            pending.borrow_mut().push(timer);
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

/// Shows one file of the current diff, from the top.
fn show_file(window: &MainWindow, diff: &DiffSet, index: usize) {
    // A different file is a different document, so it starts at the top. Opening a gap is not
    // a different document, so it goes through rebuild_rows and keeps its place.
    window.set_shared_scroll_y(0.0);
    window.set_shared_scroll_x(0.0);
    rebuild_rows(window, diff, index);
}

/// Builds the rows for one file and hands them to the view, leaving the scroll position alone.
fn rebuild_rows(window: &MainWindow, diff: &DiffSet, index: usize) {
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

    // Whether whitespace is shown changes how a line reads, not where it sits, so it belongs
    // to the rendering rather than to the layout.
    let render = RenderOptions {
        show_space_tabs: window.get_whitespace_mode() >= 1,
        show_line_endings: window.get_whitespace_mode() >= 2,
        ..Default::default()
    };
    let opts = RowOptions::default();

    // Only the arrangement on screen is laid out. Building both would flatten the whole diff
    // twice over on every file change and every toggle, for rows nobody sees.
    if window.get_side_by_side() {
        let layout = build_split(file, &opts);
        let left = RowModel::new(&layout, file, &render, Pane::Left);
        let right = RowModel::new(&layout, file, &render, Pane::Right);
        // One count for both panes, or their scrollable widths differ and the sides drift.
        // Each pane only measured its own side, and the widest line may be on either.
        window.set_split_columns(
            left.longest_line_columns()
                .max(right.longest_line_columns()),
        );
        window.set_left_rows(left.model());
        window.set_right_rows(right.model());
    } else {
        let layout = build_inline(file, &opts);
        let inline = RowModel::new(&layout, file, &render, Pane::Inline);
        window.set_inline_columns(inline.longest_line_columns());
        window.set_inline_rows(inline.model());
    }
}

/// Reads a run of lines from the right-hand file, standing in for whatever a real host does.
///
/// Only some samples come with the file the diff was taken against, and a diff named on the
/// command line never does, so asking for lines that are not here is a normal outcome rather
/// than a bug. It is also the only way to reach the failure path the view has to show.
fn fetch(sample: Option<usize>, right_start: u32, count: u32) -> Result<Vec<String>, String> {
    let name = SAMPLES
        .get(sample.ok_or("a diff read from a file does not come with the file it describes")?)
        .ok_or("no such sample")?
        .0;
    let source = SAMPLE_SOURCES
        .iter()
        .find(|(sample, _)| *sample == name)
        .map(|(_, text)| *text)
        .ok_or("this sample was built without the file it came from")?;

    let lines: Vec<String> = source
        .lines()
        .skip(right_start.max(1) as usize - 1)
        .take(count as usize)
        .map(str::to_owned)
        .collect();

    if lines.len() as u32 != count {
        return Err(format!(
            "wanted {count} lines from {right_start}, found {}",
            lines.len()
        ));
    }
    Ok(lines)
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
