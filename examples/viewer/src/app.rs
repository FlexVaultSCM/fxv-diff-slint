//! The application: what it is showing, and everything that changes it.
//!
//! One handle holds the shared state and every callback clones it, so a callback body says what
//! it does rather than restating what it needs.

// == Std crates
use std::cell::RefCell;
use std::rc::Rc;

// == Internal Crates
use fxv_diff_slint::{
    build_inline, build_split, map_span, parse_unified_diff, render_diff, split_lines, DiffSet,
    DisplayColumnExtent, DisplayedRow, Document, Pane, PaneSelection, RenderOptions, RowModel,
    RowOptions, Selection,
};

// == External Crates
use slint::{Brush, Color, ComponentHandle, Global, Model, ModelRc, SharedString, VecModel};

// == Crate
use crate::find::{diff_matches, log_match, match_ranges, Find, Found, CURRENT, SEARCH};
use crate::panes::{Panes, Tab, Which};
use crate::samples::{Choice, PLAIN_FILES};
use crate::ui::{self, MainWindow};

/// Everything a callback needs, in one handle that is cheap to clone into a closure.
///
/// Every field is shared rather than owned, so cloning this shares the same state rather than
/// copying it. Slint callbacks are `'static` closures, so each one has to own what it touches;
/// without this each closure repeats a line per field and the functions behind them grow a
/// parameter per field too.
///
/// The window is held weakly. The window owns the callbacks, the callbacks own this, so a
/// strong handle here would be a cycle that never frees.
#[derive(Clone)]
pub struct App {
    pub window: slint::Weak<MainWindow>,
    pub diff: Rc<RefCell<DiffSet>>,
    pub panes: Rc<RefCell<Panes>>,
    pub finds: Rc<RefCell<Find>>,
    /// Every sample's name and text, in the order `choices` indexes them by.
    pub samples: Rc<Vec<(String, String)>>,
    pub choices: Rc<Vec<Choice>>,
}

impl App {
    /// Builds the shared state, fills the pickers, and styles this application's channels.
    ///
    /// `samples` is every diff the viewer can offer, in the order the picker lists them.
    /// `qualify` names the sample as well as the file in each entry, which only helps when
    /// there is more than one sample to tell apart.
    ///
    /// Nothing is shown yet: the callbacks have to be installed before the first file is built,
    /// or a rebuild would run against a window that cannot answer for itself.
    pub fn new(window: &MainWindow, samples: Vec<(String, String)>, qualify: bool) -> Self {
        let samples = Rc::new(samples);
        let choices = Rc::new(Choice::list(&samples, qualify));

        let labels: Vec<SharedString> = choices.iter().map(|c| c.label.as_str().into()).collect();
        window.set_diff_names(ModelRc::from(Rc::new(VecModel::from(labels))));

        let names: Vec<SharedString> = PLAIN_FILES.iter().map(|(n, _)| (*n).into()).collect();
        window.set_plain_names(ModelRc::from(Rc::new(VecModel::from(names))));

        let app = App {
            window: window.as_weak(),
            diff: Rc::new(RefCell::new(DiffSet::default())),
            panes: Rc::new(RefCell::new(Panes::default())),
            finds: Rc::new(RefCell::new(Find::default())),
            samples,
            choices,
        };
        app.style_channels();
        app
    }

    /// Gives this application's channels a brush apiece.
    ///
    /// Built here rather than in the markup so that the constants above are the only place a
    /// channel's number is written down; an array literal is positional, and a list whose order has
    /// to agree with a constant somewhere else is a mistake waiting to happen. A host with fixed
    /// channels can assign `DiffStyle.channel-backgrounds` from `.slint` instead, with no Rust.
    ///
    /// The library's own entries are read back rather than restated, so selection and marking keep
    /// whatever the style gives them.
    pub fn style_channels(&self) {
        let window = self.window();
        let style = ui::DiffStyle::get(&window);
        let mut brushes: Vec<Brush> = style.get_channel_backgrounds().iter().collect();
        brushes.resize(CURRENT.0 as usize + 1, Brush::default());

        brushes[SEARCH.0 as usize] = Brush::SolidColor(Color::from_argb_u8(166, 0xff, 0x98, 0x00));
        brushes[CURRENT.0 as usize] = Brush::SolidColor(Color::from_argb_u8(191, 0xd8, 0x1b, 0x60));

        style.set_channel_backgrounds(ModelRc::from(Rc::new(VecModel::from(brushes))));
    }

    pub fn window(&self) -> MainWindow {
        self.window.unwrap()
    }

    /// Which tab is on screen.
    fn tab(&self) -> Tab {
        Tab::at(self.window().get_current_tab())
    }

    /// Which file of the loaded diff the picker is on.
    ///
    /// The picker is flat, so its index names a sample and a file together; only the file is
    /// wanted once the sample is loaded.
    pub fn current_file(&self) -> usize {
        self.choices
            .get(self.window().get_current_diff().max(0) as usize)
            .map_or(0, |c| c.file)
    }

    /// Whether whitespace is shown changes how a line reads, not where it sits, so it belongs
    /// to the rendering rather than to the layout.
    fn render_options(&self) -> RenderOptions {
        let window = self.window();
        RenderOptions {
            show_space_tabs: window.get_whitespace_mode() >= 1,
            show_line_endings: window.get_whitespace_mode() >= 2,
            ..Default::default()
        }
    }

    /// Parses the sample a choice names and shows the file it names.
    pub fn show_choice(&self, index: usize) {
        let Some(choice) = self.choices.get(index) else {
            return;
        };
        let Some((_, text)) = self.samples.get(choice.sample) else {
            return;
        };

        // Reparsed on every pick, which also discards any gaps opened in the file being left.
        // That is a behaviour a host would have to choose deliberately; keeping them would mean
        // holding every sample parsed at once.
        match parse_unified_diff(text) {
            Ok(parsed) => *self.diff.borrow_mut() = parsed,
            Err(e) => {
                self.window().set_status(format!("{e}").into());
                *self.diff.borrow_mut() = DiffSet::default();
                return;
            }
        }
        self.show_file(choice.file);
    }

    /// Shows one file of the current diff, from the top.
    pub fn show_file(&self, index: usize) {
        // A different file is a different document, so it starts at the top. Opening a gap is
        // not a different document, so it goes through rebuild_rows and keeps its place.
        let window = self.window();
        window.set_shared_scroll_y(0.0);
        window.set_shared_scroll_x(0.0);
        self.rebuild_rows(index);
    }

    /// Shows a plain file, with no diff behind it, in the same pane the diff uses.
    pub fn show_plain(&self, index: usize) {
        let Some((_, text)) = PLAIN_FILES.get(index) else {
            return;
        };
        let window = self.window();
        let render = self.render_options();

        // One document, so every line is numbered once. `Document::ONLY` is what a viewer with
        // nothing to compare against uses: it is the file as it stands, not a former version.
        let rows: Vec<DisplayedRow> = split_lines(text)
            .enumerate()
            .map(|(i, (line, ending))| {
                DisplayedRow::line(i as u32 + 1, Document::ONLY, line, ending, &render)
            })
            .collect();

        let model = RowModel::from_rows(rows);
        window.set_plain_columns(model.longest_line_columns());
        window.set_plain_rows(model.model());
        self.panes.borrow_mut().set_plain(model);

        // Rows built from scratch carry no highlights.
        self.search_plain();
    }

    /// Builds the rows for one file and hands them to the view, leaving the scroll position
    /// alone.
    pub fn rebuild_rows(&self, index: usize) {
        let window = self.window();
        let diff = self.diff.borrow();

        {
            let mut held = self.panes.borrow_mut();
            held.clear_diff();

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

            let render = self.render_options();
            let opts = RowOptions::default();

            // Only the arrangement on screen is laid out. Building both would flatten the whole
            // diff twice over on every file change and every toggle, for rows nobody sees.
            if window.get_side_by_side() {
                let layout = build_split(file, &opts);
                let left = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Left));
                let right = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Right));
                // One count for both panes, or their scrollable widths differ and the sides
                // drift. Each pane only measured its own side, and the widest line may be on
                // either.
                window.set_split_columns(
                    left.longest_line_columns()
                        .max(right.longest_line_columns()),
                );
                window.set_left_rows(left.model());
                window.set_right_rows(right.model());
                held.set_split(left, right);
            } else {
                let layout = build_inline(file, &opts);
                let inline = RowModel::from_rows(render_diff(&layout, file, &render, Pane::Inline));
                window.set_inline_columns(inline.longest_line_columns());
                window.set_inline_rows(inline.model());
                held.set_inline(inline);
            }
        }

        // Rows built from scratch carry no highlights, so anything being searched for has to be
        // painted again.
        self.search_diff();
    }

    /// Paints every match of the search box in the diff panes and records them for stepping.
    ///
    /// A pane of a split holds one side of the file, so a match on a removed line lands in the
    /// left pane and one on an added line in the right. Each is set on its own model, which is
    /// what a channel being set per pane is for.
    pub fn search_diff(&self) {
        let window = self.window();
        let query = window.get_search_query().to_string();
        if !query.is_empty() {
            eprintln!("find: searching the diff for {query:?}");
        }
        let opts = self.render_options();
        let diff = self.diff.borrow();
        let Some(file) = diff.files.get(self.current_file()) else {
            return;
        };

        let mut all = Vec::new();
        for (which, model) in self.panes.borrow_mut().diff_panes() {
            let found = diff_matches(model, file, &opts, &query, which);
            all.extend(found.iter().map(|(row, extent)| Found {
                which,
                row: *row,
                extent: extent.clone(),
            }));
            model.set_channel(SEARCH, &found);
        }

        // Read order: down the file, and within a row the left pane before the right.
        all.sort_by_key(|f| (f.row, f.which));
        window.set_diff_match_count(all.len() as i32);
        self.finds.borrow_mut().replace(Tab::Diff, all);
        self.show_current(Tab::Diff);
    }

    /// Paints every match of the search box in the standalone pane.
    ///
    /// Searched against the file itself rather than through the rows, because a row built from
    /// a plain file keeps no way back to what it said: `source` names a line of a diff, and
    /// there is no diff here. The application knows its own content, so it searches that.
    pub fn search_plain(&self) {
        let window = self.window();
        let query = window.get_search_query().to_string();
        if !query.is_empty() {
            eprintln!("find: searching the standalone file for {query:?}");
        }
        let opts = self.render_options();
        let Some((_, text)) = PLAIN_FILES.get(window.get_current_plain().max(0) as usize) else {
            return;
        };

        let mut found = Vec::new();
        {
            let mut held = self.panes.borrow_mut();
            let Some(model) = held.get(Which::Plain) else {
                return;
            };

            // Row order is line order here, because that is how the rows were built.
            for (row, (line, _)) in split_lines(text).enumerate() {
                for chars in match_ranges(line, &query) {
                    let columns = map_span(line, chars.clone(), &opts);
                    log_match(
                        Which::Plain,
                        row,
                        Document::ONLY,
                        row as u32 + 1,
                        &chars,
                        &columns,
                    );
                    found.push((
                        row,
                        DisplayColumnExtent::Columns(columns.start as u32..columns.end as u32),
                    ));
                }
            }
            window.set_plain_match_count(found.len() as i32);
            model.set_channel(SEARCH, &found);
        }

        self.finds.borrow_mut().replace(
            Tab::Standalone,
            found
                .into_iter()
                .map(|(row, extent)| Found {
                    which: Which::Plain,
                    row,
                    extent,
                })
                .collect(),
        );
        self.show_current(Tab::Standalone);
    }

    /// Reports a finished drag.
    ///
    /// The view has drawn it already; this is the moment the gesture is worth turning into
    /// something durable. For now it only says what was selected.
    pub fn selection_finished(&self, pane: i32, selection: PaneSelection) {
        let which = match pane {
            0 => Which::Inline,
            1 => Which::Left,
            2 => Which::Right,
            _ => Which::Plain,
        };
        let selection = Selection::from(selection);
        eprintln!(
            "selection: {which:?} pane, from row {} column {} to row {} column {}",
            selection.anchor.row,
            selection.anchor.column,
            selection.focus.row,
            selection.focus.column
        );
    }

    /// Moves to another match and brings it into sight, wrapping at either end.
    ///
    /// `step` is 1 for the next match and -1 for the previous. Only the tab on screen is
    /// stepped: the other holds a different document, where this position would mean nothing.
    pub fn step_match(&self, step: isize) {
        let tab = self.tab();
        self.finds.borrow_mut().advance(tab, step);
        self.show_current(tab);
    }

    /// Paints the current match over the rest and brings it into sight.
    fn show_current(&self, tab: Tab) {
        let finds = self.finds.borrow();
        let mut held = self.panes.borrow_mut();

        // The current match moves from pane to pane, so this tab's panes are cleared before one
        // of them is given the new mark.
        held.clear_channel(tab, CURRENT);

        let Some(found) = finds.current(tab) else {
            // No matches, so nothing is current and nothing is revealed.
            self.set_current(tab, 0, -1);
            return;
        };

        if let Some(model) = held.get(found.which) {
            model.set_channel(CURRENT, &[(found.row, found.extent.clone())]);
        }
        eprintln!(
            "find: at match {} of {}, row {} of the {:?} pane",
            finds.at(tab) + 1,
            finds.matches(tab).len(),
            found.row,
            found.which
        );

        self.set_current(tab, finds.at(tab) as i32 + 1, found.row as i32);
    }

    /// Tells the window which match is current and which row to reveal.
    fn set_current(&self, tab: Tab, at: i32, row: i32) {
        let window = self.window();
        match tab {
            Tab::Standalone => {
                window.set_plain_match_at(at);
                window.set_reveal_plain_row(row);
            }
            Tab::Diff => {
                window.set_diff_match_at(at);
                window.set_reveal_diff_row(row);
            }
        }
        // Bumped every time, so stepping back to the match already named still scrolls to it
        // after the view has been moved by hand.
        window.set_reveal_token(window.get_reveal_token().wrapping_add(1));
    }
}
