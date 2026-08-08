// == Std crates
use std::cell::RefCell;
use std::env;
use std::fs;
use std::rc::Rc;
use std::time;

// == Internal Crates
use fxv_diff_slint::render_diff;
use fxv_diff_slint::split_lines;
use fxv_diff_slint::{
    build_inline, build_split, parse_unified_diff, DiffSet, DisplayedRow, FileDiff, Pane,
    RenderOptions, RowModel, RowOptions, Side,
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
const SAMPLES: &[(&str, &str)] = &[
    (
        "Gaps and tabs",
        include_str!("../samples/synthetic_gaps.diff"),
    ),
    (
        "Add parsing (15 files)",
        include_str!("../samples/add_parsing_commit.diff"),
    ),
    (
        "Adds, renames, binary",
        include_str!("../samples/mixed_operations.diff"),
    ),
];

/// The right-hand file a sample's gaps can be opened from, where we have it. Without one the
/// gaps are still shown, but there is nothing to fill them with.
const SAMPLE_SOURCES: &[(&str, &str)] = &[(
    "Gaps and tabs",
    include_str!("../samples/synthetic_gaps.after.txt"),
)];

/// Plain files for the standalone tab, which shows a file with no diff behind it.
///
/// Two of the widget's own sources, so the content is real, plus the file the gaps sample was
/// taken against, which carries a tab and some lines wide enough to need scrolling.
const PLAIN_FILES: &[(&str, &str)] = &[
    (
        "synthetic_gaps.after.txt",
        include_str!("../samples/synthetic_gaps.after.txt"),
    ),
    (
        "span.rs",
        include_str!("../../../crates/fxv-diff-slint/src/span.rs"),
    ),
    (
        "selection.rs",
        include_str!("../../../crates/fxv-diff-slint/src/selection.rs"),
    ),
];

/// One entry of the diff picker: a single file, named together with the diff it came from.
///
/// The picker is flat rather than a sample chooser feeding a file chooser, so reaching a file is
/// one selection. That costs parsing every sample at startup to find out what is in it, which is
/// nothing at this size.
struct Choice {
    label: String,
    sample: usize,
    file: usize,
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let diff = Rc::new(RefCell::new(DiffSet::default()));

    // A path on the command line replaces the built-in samples.
    let from_file = env::args().nth(1).map(|path| {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
        (path, text)
    });
    let samples: Rc<Vec<(String, String)>> = Rc::new(match &from_file {
        Some((path, text)) => vec![(path.clone(), text.clone())],
        None => SAMPLES
            .iter()
            .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
            .collect(),
    });
    // Naming the sample as well as the file only helps when there is more than one to tell
    // apart, and a diff read from a file is named after itself already.
    let choices = Rc::new(enumerate(&samples, from_file.is_none()));

    let labels: Vec<SharedString> = choices.iter().map(|c| c.label.as_str().into()).collect();
    window.set_diff_names(ModelRc::from(Rc::new(VecModel::from(labels))));

    let names: Vec<SharedString> = PLAIN_FILES.iter().map(|(n, _)| (*n).into()).collect();
    window.set_plain_names(ModelRc::from(Rc::new(VecModel::from(names))));

    {
        let window = window.as_weak();
        let diff = diff.clone();
        let samples = samples.clone();
        let choices = choices.clone();
        window.unwrap().on_diff_changed(move |index| {
            show_choice(
                &window.unwrap(),
                &diff,
                &samples,
                &choices,
                index.max(0) as usize,
            );
        });
    }

    {
        let window = window.as_weak();
        window
            .unwrap()
            .on_plain_changed(move |index| show_plain(&window.unwrap(), index.max(0) as usize));
    }

    {
        let window = window.as_weak();
        let diff = diff.clone();
        let choices = choices.clone();
        // One timer per request, kept alive past the closure that started it, since a dropped
        // Timer never fires. Sharing one would cancel whichever gap was already waiting and
        // leave it saying "loading" for good.
        let pending: Rc<RefCell<Vec<Timer>>> = Rc::new(RefCell::new(Vec::new()));
        window.unwrap().on_gap_expand_requested(move |request| {
            let w = window.unwrap();
            let Some(choice) = choices.get(w.get_current_diff().max(0) as usize) else {
                return;
            };
            let index = choice.file;
            let source = source_for(choice.sample);
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
                        match fetch(source, start, count) {
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

    // Layout changes what has to be built, so it goes back through show_file rather than
    // restyling what is already there. Only the diff has a layout to change.
    {
        let window = window.as_weak();
        let diff = diff.clone();
        let choices = choices.clone();
        window.unwrap().on_layout_changed(move || {
            let w = window.unwrap();
            let file = choices
                .get(w.get_current_diff().max(0) as usize)
                .map_or(0, |c| c.file);
            show_file(&w, &diff.borrow(), file);
        });
    }

    // Whitespace applies to any text, so it rebuilds both tabs. Neither is expensive enough to
    // be worth tracking which one is on screen.
    {
        let window = window.as_weak();
        let diff = diff.clone();
        let choices = choices.clone();
        window.unwrap().on_whitespace_changed(move || {
            let w = window.unwrap();
            let file = choices
                .get(w.get_current_diff().max(0) as usize)
                .map_or(0, |c| c.file);
            show_file(&w, &diff.borrow(), file);
            show_plain(&w, w.get_current_plain().max(0) as usize);
        });
    }

    show_choice(&window, &diff, &samples, &choices, 0);
    show_plain(&window, 0);

    window.run()
}

/// Lists every file of every sample as one entry, parsing each sample to find out what is in it.
fn enumerate(samples: &[(String, String)], qualify: bool) -> Vec<Choice> {
    let mut choices = Vec::new();
    for (sample, (name, text)) in samples.iter().enumerate() {
        let Ok(parsed) = parse_unified_diff(text) else {
            // A sample that will not parse still deserves an entry, so selecting it shows the
            // parse error rather than the file silently not being in the list.
            choices.push(Choice {
                label: name.clone(),
                sample,
                file: 0,
            });
            continue;
        };
        for (file, diff) in parsed.files.iter().enumerate() {
            let described = describe(diff);
            choices.push(Choice {
                label: if qualify {
                    format!("{name}: {described}")
                } else {
                    described
                },
                sample,
                file,
            });
        }
    }
    choices
}

/// Parses the sample a choice names, if it is not already loaded, and shows the file it names.
fn show_choice(
    window: &MainWindow,
    diff: &RefCell<DiffSet>,
    samples: &[(String, String)],
    choices: &[Choice],
    index: usize,
) {
    let Some(choice) = choices.get(index) else {
        return;
    };
    let Some((_, text)) = samples.get(choice.sample) else {
        return;
    };

    // Reparsed on every pick, which also discards any gaps opened in the file being left. That
    // is the behaviour a host would have to choose deliberately; keeping them would mean holding
    // every sample parsed at once.
    match parse_unified_diff(text) {
        Ok(parsed) => *diff.borrow_mut() = parsed,
        Err(e) => {
            window.set_status(format!("{e}").into());
            *diff.borrow_mut() = DiffSet::default();
            return;
        }
    }
    show_file(window, &diff.borrow(), choice.file);
}

/// Shows one file of the current diff, from the top.
fn show_file(window: &MainWindow, diff: &DiffSet, index: usize) {
    // A different file is a different document, so it starts at the top. Opening a gap is not
    // a different document, so it goes through rebuild_rows and keeps its place.
    window.set_shared_scroll_y(0.0);
    window.set_shared_scroll_x(0.0);
    rebuild_rows(window, diff, index);
}

/// Shows a plain file, with no diff behind it, in the same pane the diff uses.
fn show_plain(window: &MainWindow, index: usize) {
    let Some((_, text)) = PLAIN_FILES.get(index) else {
        return;
    };
    let render = render_options(window);

    // One document, so every line is numbered once. `Side::Right` is what a viewer with nothing
    // to compare against uses: it is the file as it stands, not a former version of one.
    let rows: Vec<DisplayedRow> = split_lines(text)
        .enumerate()
        .map(|(i, (line, ending))| {
            DisplayedRow::line(i as u32 + 1, Side::Right, line, ending, &render)
        })
        .collect();

    let model = RowModel::from_rows(rows);
    window.set_plain_columns(model.longest_line_columns());
    window.set_plain_rows(model.model());
}

/// Whether whitespace is shown changes how a line reads, not where it sits, so it belongs to the
/// rendering rather than to the layout.
fn render_options(window: &MainWindow) -> RenderOptions {
    RenderOptions {
        show_space_tabs: window.get_whitespace_mode() >= 1,
        show_line_endings: window.get_whitespace_mode() >= 2,
        ..Default::default()
    }
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

    let render = render_options(window);
    let opts = RowOptions::default();

    // Only the arrangement on screen is laid out. Building both would flatten the whole diff
    // twice over on every file change and every toggle, for rows nobody sees.
    if window.get_side_by_side() {
        let layout = build_split(file, &opts);
        let left = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Left));
        let right = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Right));
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
        let inline = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Inline));
        window.set_inline_columns(inline.longest_line_columns());
        window.set_inline_rows(inline.model());
    }
}

/// The file a sample's gaps can be filled from, if it came with one.
fn source_for(sample: usize) -> Option<&'static str> {
    let name = &SAMPLES.get(sample)?.0;
    SAMPLE_SOURCES
        .iter()
        .find(|(s, _)| s == name)
        .map(|(_, text)| *text)
}

/// Reads a run of lines from the right-hand file, standing in for whatever a real host does.
///
/// Only some samples come with the file the diff was taken against, and a diff named on the
/// command line never does, so asking for lines that are not here is a normal outcome rather
/// than a bug. It is also the only way to reach the failure path the view has to show.
fn fetch(source: Option<&str>, right_start: u32, count: u32) -> Result<Vec<String>, String> {
    let source = source.ok_or("this sample was built without the file it came from")?;

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
