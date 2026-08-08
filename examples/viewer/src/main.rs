//! A viewer for the diff widget, and the worked example of driving it from a host.

// == Std crates
use std::cell::RefCell;
use std::env;
use std::fs;
use std::rc::Rc;
use std::time;

// == External Crates
use slint::{ComponentHandle, Timer};

// == Crate
mod app;
mod find;
mod panes;
mod samples;

use app::App;
use samples::{fetch, SAMPLES};

// Machine-generated; see the note on the library's `ui` module.
//
// dead_code because a consumer re-parses the library's .slint sources and re-embeds the images
// they reference, while the globals holding them come from the library crate. The duplicates
// are never read, and nothing here can stop them being emitted.
#[allow(clippy::absolute_paths, dead_code)]
pub mod ui {
    slint::include_modules!();
}
use ui::MainWindow;

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    // A path on the command line replaces the built-in samples.
    let from_file = env::args().nth(1).map(|path| {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
        (path, text)
    });
    let samples: Vec<(String, String)> = match &from_file {
        Some((path, text)) => vec![(path.clone(), text.clone())],
        None => SAMPLES
            .iter()
            .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
            .collect(),
    };
    // A diff read from a file is named after itself already, so its one entry needs no
    // sample name in front of it.
    let app = App::new(&window, samples, from_file.is_none());

    {
        let app = app.clone();
        window.on_diff_changed(move |index| app.show_choice(index.max(0) as usize));
    }

    {
        let app = app.clone();
        window.on_plain_changed(move |index| app.show_plain(index.max(0) as usize));
    }

    {
        let app = app.clone();
        // One timer per request, kept alive past the closure that started it, since a dropped
        // Timer never fires. Sharing one would cancel whichever gap was already waiting and
        // leave it saying "loading" for good.
        let pending: Rc<RefCell<Vec<Timer>>> = Rc::new(RefCell::new(Vec::new()));
        window.on_gap_expand_requested(move |request| {
            let Some(choice) = app
                .choices
                .get(app.window().get_current_diff().max(0) as usize)
            else {
                return;
            };
            let index = choice.file;
            let source = choice.source();
            let start = request.right_start.max(1) as u32;
            let count = request.count.max(0) as u32;

            if let Some(file) = app.diff.borrow_mut().files.get_mut(index) {
                // A new attempt supersedes whatever the last one said.
                file.clear_failed_fetches();
                file.fetch_started(start, count);
            }
            app.rebuild_rows(index);

            // A real host reads another revision of the file, over a network as often as not.
            // Answering on a timer instead of straight away is what makes the waiting and
            // failure states reachable here at all.
            let app = app.clone();
            let left = request.left_start.max(1) as u32;
            let timer = Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                time::Duration::from_millis(600),
                move || {
                    if let Some(file) = app.diff.borrow_mut().files.get_mut(index) {
                        match fetch(source, start, count) {
                            // The request names a run on each side, and the lines are the same
                            // either way, so both numbers go in.
                            Ok(lines) => file.expand(left, start, lines),
                            Err(why) => file.fetch_failed(start, count, why),
                        }
                    }
                    app.rebuild_rows(index);
                },
            );
            pending.borrow_mut().push(timer);
        });
    }

    // Layout changes what has to be built, so it goes back through show_file rather than
    // restyling what is already there. Only the diff has a layout to change.
    {
        let app = app.clone();
        window.on_layout_changed(move || app.show_file(app.current_file()));
    }

    // Whitespace applies to any text, so it rebuilds both tabs. Neither is expensive enough to
    // be worth tracking which one is on screen.
    {
        let app = app.clone();
        window.on_whitespace_changed(move || {
            app.show_file(app.current_file());
            app.show_plain(app.window().get_current_plain().max(0) as usize);
        });
    }

    // Searching repaints a channel over the rows that are already built, so unlike whitespace
    // or layout it costs no rebuild.
    {
        let app = app.clone();
        window.on_search_changed(move || {
            app.search_diff();
            app.search_plain();
        });
    }

    // Stepping between matches, which repaints one channel and scrolls; no rebuild.
    for (install, step) in [
        (MainWindow::on_find_next as fn(&MainWindow, _), 1_isize),
        (MainWindow::on_find_previous as fn(&MainWindow, _), -1),
    ] {
        let app = app.clone();
        install(&window, Box::new(move || app.step_match(step)));
    }

    app.style_channels();
    app.show_choice(0);
    app.show_plain(0);

    window.run()
}
