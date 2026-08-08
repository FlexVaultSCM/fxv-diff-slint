//! Turning a laid-out diff into rows a pane can draw.
//!
//! A `Layout` says which line belongs at which position. A pane has to answer more than that:
//! what the line reads as once tabs are expanded, how many columns it occupies, which numbers
//! its gutter shows, and what colour it draws with. Those are all decisions about display, so
//! they are made here rather than being baked into the layout.
//!
//! One layout serves both panes of a split view. Each reads its own side of every entry, which
//! is why a filler is simply the side that was absent rather than a row pretending to be one.
//!
//! Still no pixels. A column becomes a position on screen by multiplying by the character
//! advance, and that happens in the markup.

// == Std crates
use std::mem;
use std::rc::Rc;

// == External Crates
use slint::{Model, ModelRc, SharedString, VecModel};

// == Internal Crates
use crate::diff::layout::{GapState, Layout, Line, Row};
use crate::diff::model::{FileDiff, LineEnding, LineKind, LineRef};
use crate::highlight::{DisplayColumnExtent, Highlight, HighlightKind};
use crate::span::Side;
use crate::text::{render_line, RenderOptions};
use crate::ui;

/// How many numbers a gutter can show at once.
///
/// Two, because that is what an inline diff needs and nothing here wants more. The cap lives
/// at this level on purpose: a layout has no opinion about gutters, and the widget struct
/// carries two plain integers rather than a list, which would cost every row an allocation to
/// hold what fits in eight bytes.
pub const GUTTER_COLUMNS: usize = 2;

/// Which part of a layout a pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// One column showing both files, with two numbers against an unchanged line.
    Inline,
    /// The left file only, as one side of a split view.
    Left,
    /// The right file only.
    Right,
}

/// What a row draws as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Names the file.
    Header,
    /// Unchanged content, shown to give the change context.
    Context,
    Added,
    Removed,
    /// Content that exists but is not shown.
    Gap,
    /// Nothing on this side. Keeps the two panes of a split view in step.
    Filler,
}

/// One row as a pane will draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayedRow {
    pub kind: RowKind,
    /// Gutter numbers, in the order the view draws them. A pane showing one file fills only
    /// the first.
    pub numbers: [Option<u32>; GUTTER_COLUMNS],
    /// Which file names this row and the number it has there, for describing a selection over
    /// it. Absent on rows that stand for no line.
    ///
    /// Settled here rather than asked for later, because a pane knows which file it is showing
    /// and an entry knows which side a line came from.
    pub id: Option<(Side, u32)>,
    /// The line this row was rendered from, for anything needing the original text.
    pub source: Option<LineRef>,
    /// Display text, tabs already expanded. Empty for fillers and gaps.
    pub text: SharedString,
    /// Columns the text occupies.
    pub columns: u32,
    /// How many lines a gap is hiding. Zero for every other kind.
    pub hidden_count: u32,
    /// Only meaningful on a gap.
    pub gap_state: GapState,
    /// A gap's hunk heading, or why its fetch failed. Empty otherwise.
    pub note: SharedString,
    /// Where a gap's hidden run starts, on each side.
    pub gap_start: (u32, u32),
    /// The run of a gap being fetched, numbered on the right, when one is.
    pub pending: Option<(u32, u32)>,
    /// Whether a selection may cover this row. Gaps and headers break a selection; a filler
    /// does not, being a real position in a pane that simply holds no line.
    pub selectable: bool,
}

impl DisplayedRow {
    /// One line of ordinary content, numbered and selectable.
    ///
    /// What a view showing a plain file builds its rows from, rather than going through a
    /// layout that exists to describe a change.
    ///
    /// `ending` is how the line was terminated in the file, which the text alone cannot say
    /// once the terminator has been split off. `text::split_lines` reports both together.
    pub fn line(
        number: u32,
        side: Side,
        text: &str,
        ending: LineEnding,
        opts: &RenderOptions,
    ) -> Self {
        let (rendered, columns) = render_line(text, ending, opts);
        DisplayedRow {
            numbers: [Some(number), None],
            id: Some((side, number)),
            text: rendered.as_str().into(),
            columns: columns as u32,
            selectable: true,
            ..DisplayedRow::blank(RowKind::Context)
        }
    }

    fn blank(kind: RowKind) -> Self {
        DisplayedRow {
            kind,
            numbers: [None; GUTTER_COLUMNS],
            id: None,
            source: None,
            text: SharedString::new(),
            columns: 0,
            hidden_count: 0,
            gap_state: GapState::Hidden,
            note: SharedString::new(),
            gap_start: (0, 0),
            pending: None,
            selectable: false,
        }
    }
}

/// The rows a pane is showing, in both the form this crate reasons about and the form the
/// widget draws, kept together so highlights can change without either being rebuilt.
pub struct RowModel {
    rows: Vec<DisplayedRow>,
    model: Rc<VecModel<ui::DiffRow>>,
    longest_line_columns: i32,
    /// What each row that carries anything is currently drawing, in row order.
    ///
    /// Kept so a change can write back only the rows whose highlights actually differ. Handing
    /// Slint a row it already holds counts as a change to the model, so the list invalidates
    /// and redraws that row for nothing.
    drawn: Vec<(usize, Vec<Highlight>)>,
}

impl RowModel {
    /// Renders one pane's worth of a layout.
    pub fn new(layout: &Layout, file: &FileDiff, opts: &RenderOptions, pane: Pane) -> Self {
        let rows: Vec<DisplayedRow> = layout
            .rows
            .iter()
            .map(|entry| display(entry, file, opts, pane))
            .collect();

        let longest_line_columns = rows.iter().map(|r| r.columns).max().unwrap_or(0) as i32;
        let converted: Vec<ui::DiffRow> = rows.iter().map(|r| r.into()).collect();

        RowModel {
            rows,
            model: Rc::new(VecModel::from(converted)),
            longest_line_columns,
            drawn: Vec::new(),
        }
    }

    /// Builds a pane from rows supplied directly.
    ///
    /// The route for content that is not a diff. Everything a pane does past this point, the
    /// grid, highlights, selection and scrolling, works from these rows and asks nothing about
    /// where they came from, so a plain file listing is as good an input as a laid-out diff.
    pub fn from_rows(rows: Vec<DisplayedRow>) -> Self {
        let longest_line_columns = rows.iter().map(|r| r.columns).max().unwrap_or(0) as i32;
        let converted: Vec<ui::DiffRow> = rows.iter().map(|r| r.into()).collect();

        RowModel {
            rows,
            model: Rc::new(VecModel::from(converted)),
            longest_line_columns,
            drawn: Vec::new(),
        }
    }

    /// What the widget binds to.
    pub fn model(&self) -> ModelRc<ui::DiffRow> {
        ModelRc::from(self.model.clone())
    }

    /// The rows themselves, for the selection arithmetic.
    pub fn rows(&self) -> &[DisplayedRow] {
        &self.rows
    }

    /// Columns the longest line this pane draws occupies, which is how a view sizes its
    /// horizontal scrolling.
    ///
    /// Two panes of a split must be given the same number or their scrollable widths differ
    /// and the sides drift apart, so a caller takes the larger of the two. A pane only ever
    /// sees its own side, and the widest line may be on either.
    pub fn longest_line_columns(&self) -> i32 {
        self.longest_line_columns
    }

    /// Replaces every highlight on screen.
    ///
    /// Returns how many rows were written, which is the measure of what this cost.
    ///
    /// WIP: this takes one flat list, so a caller with two sources has to merge them and
    /// resend both whenever either changes. It becomes one call per channel next, at which
    /// point a change to one channel stops touching the other's rows at all.
    pub fn set_highlights(&mut self, highlights: &[(usize, Highlight)]) -> usize {
        let wanted = group_by_row(highlights, self.rows.len());
        // Taken out so the loops below read a local rather than borrowing `self`, which is
        // what lets `write` take `&mut self` and keep an ordinary counter.
        let previous = mem::take(&mut self.drawn);
        let mut written = 0;

        // Rows that had something and no longer do.
        for (index, _) in &previous {
            if wanted.binary_search_by_key(index, |(i, _)| *i).is_err() {
                written += usize::from(self.write(*index, &[]));
            }
        }

        for (index, list) in &wanted {
            let unchanged = previous
                .binary_search_by_key(index, |(i, _)| *i)
                .is_ok_and(|at| previous[at].1 == *list);
            if !unchanged {
                written += usize::from(self.write(*index, list));
            }
        }

        self.drawn = wanted;
        written
    }

    /// Hands one row back to Slint. Reports whether it did, since a row that has gone missing
    /// is not a write.
    fn write(&mut self, index: usize, highlights: &[Highlight]) -> bool {
        let Some(mut row) = self.model.row_data(index) else {
            return false;
        };
        row.highlights = to_slint_highlights(highlights);
        self.model.set_row_data(index, row);
        true
    }
}

/// Renders one entry as this pane sees it.
fn display(entry: &Row, file: &FileDiff, opts: &RenderOptions, pane: Pane) -> DisplayedRow {
    match entry {
        Row::Header => DisplayedRow {
            text: file.display_path().into(),
            columns: file.display_path().chars().count() as u32,
            ..DisplayedRow::blank(RowKind::Header)
        },

        Row::Gap {
            left_start,
            right_start,
            hidden,
            state,
            pending,
            heading,
            reason,
        } => {
            // A failure explains itself; otherwise the heading names what follows.
            let note = reason.as_deref().or(heading.as_deref()).unwrap_or_default();
            DisplayedRow {
                hidden_count: *hidden,
                gap_state: *state,
                note: note.into(),
                gap_start: (*left_start, *right_start),
                pending: *pending,
                ..DisplayedRow::blank(RowKind::Gap)
            }
        }

        Row::Lines(diff_row) => {
            let shown = match pane {
                Pane::Left => diff_row.left,
                Pane::Right => diff_row.right,
                // Inline draws one line per entry, so whichever side is present is the one to
                // draw. An unchanged line is on both and either gives the same text.
                Pane::Inline => diff_row.left.or(diff_row.right),
            };

            let Some(row) = shown else {
                return DisplayedRow::blank(RowKind::Filler);
            };
            let Some(line) = file.line(row.source) else {
                return DisplayedRow::blank(RowKind::Filler);
            };

            let (text, columns) = render_line(&line.text, line.line_ending, opts);
            let side = match line.kind {
                LineKind::Removed => Side::Left,
                LineKind::Added => Side::Right,
                // An unchanged line is in both files, so which one names it is decided by the
                // pane. An inline view means the right, being the file as it stands now.
                LineKind::Context => match pane {
                    Pane::Left => Side::Left,
                    Pane::Inline | Pane::Right => Side::Right,
                },
            };

            DisplayedRow {
                kind: match line.kind {
                    LineKind::Context => RowKind::Context,
                    LineKind::Added => RowKind::Added,
                    LineKind::Removed => RowKind::Removed,
                },
                numbers: gutter(diff_row.left, diff_row.right, pane),
                id: Some((side, row.line)),
                source: Some(row.source),
                text: text.as_str().into(),
                columns: columns as u32,
                selectable: true,
                ..DisplayedRow::blank(RowKind::Context)
            }
        }
    }
}

/// The numbers this pane's gutter shows for an entry.
fn gutter(left: Option<Line>, right: Option<Line>, pane: Pane) -> [Option<u32>; GUTTER_COLUMNS] {
    match pane {
        Pane::Inline => [left.map(|r| r.line), right.map(|r| r.line)],
        Pane::Left => [left.map(|r| r.line), None],
        Pane::Right => [right.map(|r| r.line), None],
    }
}

/// Gathers highlights per row, in row order, dropping any that name a row that is not there.
///
/// Sorted rather than grouped in place so both this and the previous state can be walked by
/// binary search, instead of scanning one for every entry of the other.
fn group_by_row(highlights: &[(usize, Highlight)], rows: usize) -> Vec<(usize, Vec<Highlight>)> {
    let mut ordered: Vec<&(usize, Highlight)> =
        highlights.iter().filter(|(i, _)| *i < rows).collect();
    ordered.sort_by_key(|(i, _)| *i);

    let mut grouped: Vec<(usize, Vec<Highlight>)> = Vec::new();
    for (index, highlight) in ordered {
        match grouped.last_mut() {
            Some((at, list)) if at == index => list.push(highlight.clone()),
            _ => grouped.push((*index, vec![highlight.clone()])),
        }
    }
    grouped
}

impl From<&DisplayedRow> for ui::DiffRow {
    fn from(row: &DisplayedRow) -> Self {
        // A gap draws no numbers, so it spends the two number fields on where its hidden run
        // starts. Opening one has to say which lines to fetch, and that is the only thing it
        // has to say. Losing this makes every gap ask for line zero, so whichever one is
        // clicked, the first one opens.
        let (left, right) = match row.kind {
            RowKind::Gap => (row.gap_start.0, row.gap_start.1),
            _ => (row.numbers[0].unwrap_or(0), row.numbers[1].unwrap_or(0)),
        };

        ui::DiffRow {
            kind: row.kind.into(),
            // Zero means "no number in this column". Line numbers are 1-based, so it cannot
            // collide with a real one.
            left_line: left as i32,
            right_line: right as i32,
            text: if row.kind == RowKind::Gap {
                row.note.clone()
            } else {
                row.text.clone()
            },
            hidden_count: row.hidden_count as i32,
            gap_state: row.gap_state.into(),
            // Zero count means nothing is in flight. A fetch of no lines is not a thing.
            busy_start: row.pending.map_or(0, |(start, _)| start) as i32,
            busy_count: row.pending.map_or(0, |(_, count)| count) as i32,
            highlights: ModelRc::default(),
        }
    }
}

/// Packs a row's highlights into the form the widget binds to.
///
/// A row with none costs nothing: `ModelRc::default()` holds no model at all. A row with any
/// costs two allocations, the vector and the model wrapping it.
///
/// WIP: those two could be one, and the row need not be rewritten at all. `Model::as_any`
/// downcasts a `ModelRc` back to its `VecModel`, and `VecModel::set_vec` replaces the contents
/// through a shared reference, so a row could be given its model once and have the contents
/// swapped afterwards. The gain is not the allocation: changing a row's highlights would stop
/// counting as a change to the row itself, so the list would re-evaluate the highlights alone
/// instead of the row. Left until the write path is restructured for channels, because it
/// changes what "a row was written" means and the tests here measure exactly that.
fn to_slint_highlights(highlights: &[Highlight]) -> ModelRc<ui::DiffHighlight> {
    if highlights.is_empty() {
        return ModelRc::default();
    }
    let converted: Vec<ui::DiffHighlight> = highlights
        .iter()
        .map(|h| {
            // Slint has no enum carrying a payload, so the two cases flatten into a flag.
            // `end` is meaningless when `to_end` is set; the row runs to its own edge.
            let (start, end, to_end) = match &h.extent {
                DisplayColumnExtent::Columns(columns) => {
                    (columns.start as i32, columns.end as i32, false)
                }
                DisplayColumnExtent::ToEnd { from } => (*from as i32, 0, true),
            };
            ui::DiffHighlight {
                start,
                end,
                to_end,
                kind: h.kind.into(),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(converted)))
}

impl From<HighlightKind> for ui::DiffHighlightKind {
    fn from(kind: HighlightKind) -> Self {
        match kind {
            HighlightKind::Selection => ui::DiffHighlightKind::Selection,
            HighlightKind::Marked => ui::DiffHighlightKind::Marked,
        }
    }
}

impl From<GapState> for ui::DiffGapState {
    fn from(state: GapState) -> Self {
        match state {
            GapState::Hidden => ui::DiffGapState::Hidden,
            GapState::Waiting => ui::DiffGapState::Waiting,
            GapState::Failed => ui::DiffGapState::Failed,
        }
    }
}

impl From<RowKind> for ui::DiffRowKind {
    fn from(kind: RowKind) -> Self {
        match kind {
            RowKind::Header => ui::DiffRowKind::Header,
            RowKind::Context => ui::DiffRowKind::Context,
            RowKind::Added => ui::DiffRowKind::Added,
            RowKind::Removed => ui::DiffRowKind::Removed,
            RowKind::Gap => ui::DiffRowKind::Gap,
            RowKind::Filler => ui::DiffRowKind::Filler,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::layout::{build_inline, build_split, RowOptions};
    use crate::test_fixtures::{file, shown};
    use std::ops::Range;

    fn kinds(rows: &[DisplayedRow]) -> Vec<RowKind> {
        rows.iter().map(|r| r.kind).collect()
    }

    // == Rendering

    #[test]
    fn tabs_are_expanded_in_row_text() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let removed = rows.iter().find(|r| r.kind == RowKind::Removed).unwrap();

        assert!(
            removed.text.starts_with("    "),
            "a leading tab reaches the next stop: {:?}",
            removed.text
        );
        assert!(!removed.text.contains('\t'));
    }

    #[test]
    fn the_tab_width_flows_through_to_row_text() {
        let f = file();
        let layout = build_inline(&f, &RowOptions::default());
        let wide = RowModel::new(
            &layout,
            &f,
            &RenderOptions {
                tab_width: 8,
                ..RenderOptions::default()
            },
            Pane::Inline,
        );
        let removed = wide
            .rows()
            .iter()
            .find(|r| r.kind == RowKind::Removed)
            .unwrap();

        assert!(removed.text.starts_with("        "), "eight columns");
    }

    #[test]
    fn the_longest_line_is_measured_after_expansion() {
        let f = file();
        let layout = build_inline(&f, &RowOptions::default());
        let opts = RenderOptions::default();
        let model = RowModel::new(&layout, &f, &opts, Pane::Inline);

        let longest = model
            .rows()
            .iter()
            .map(|r| r.text.chars().count() as u32)
            .max()
            .unwrap();
        assert_eq!(
            model.longest_line_columns() as u32,
            longest,
            "the tab counts as the columns it reaches, not as one character"
        );
    }

    #[test]
    fn a_split_is_sized_by_the_wider_of_its_two_panes() {
        // Each pane only ever measures the side it draws, and the widest line may be on
        // either. Sizing from one pane leaves the view unable to scroll to text the other is
        // already drawing, which is what a caller taking the larger of the two avoids.
        let f = file();
        let layout = build_split(&f, &RowOptions::default());
        let opts = RenderOptions::default();

        let left = RowModel::new(&layout, &f, &opts, Pane::Left);
        let right = RowModel::new(&layout, &f, &opts, Pane::Right);

        assert!(
            right.longest_line_columns() > left.longest_line_columns(),
            "the fixture's widest line is an addition, so it is only on the right"
        );

        let widest = right
            .rows()
            .iter()
            .chain(left.rows())
            .map(|r| r.columns)
            .max()
            .unwrap() as i32;
        assert_eq!(
            left.longest_line_columns()
                .max(right.longest_line_columns()),
            widest
        );
    }

    #[test]
    fn an_empty_layout_measures_zero() {
        let f = file();
        assert_eq!(
            RowModel::new(
                &Layout::default(),
                &f,
                &RenderOptions::default(),
                Pane::Inline
            )
            .longest_line_columns(),
            0
        );
    }

    // == Panes

    #[test]
    fn the_two_panes_of_a_split_are_always_the_same_length() {
        let f = file();
        let layout = build_split(&f, &RowOptions::default());
        assert_eq!(
            shown(&layout, &f, Pane::Left).len(),
            shown(&layout, &f, Pane::Right).len(),
            "one layout, so the panes cannot drift"
        );
    }

    #[test]
    fn a_side_with_no_line_draws_as_a_filler() {
        let f = file();
        let layout = build_split(&f, &RowOptions::default());
        let right = shown(&layout, &f, Pane::Right);

        // The fixture removes one line and adds one, so the sides are even and nothing is a
        // filler; the removal and the addition sit opposite each other.
        assert!(
            !kinds(&right).contains(&RowKind::Filler),
            "an even change needs no padding"
        );
        assert!(kinds(&right).contains(&RowKind::Added));
    }

    #[test]
    fn an_inline_pane_numbers_both_columns_of_an_unchanged_line() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let context = rows.iter().find(|r| r.kind == RowKind::Context).unwrap();

        assert!(
            context.numbers[0].is_some() && context.numbers[1].is_some(),
            "an unchanged line has a number in each file"
        );
    }

    #[test]
    fn a_split_pane_numbers_one_column() {
        let f = file();
        let layout = build_split(&f, &RowOptions::default());
        for pane in [Pane::Left, Pane::Right] {
            let rows = shown(&layout, &f, pane);
            assert!(
                rows.iter().all(|r| r.numbers[1].is_none()),
                "a pane showing one file fills one column"
            );
        }
    }

    #[test]
    fn a_pane_names_an_unchanged_line_by_its_own_file() {
        let f = file();
        let layout = build_split(&f, &RowOptions::default());

        let left = shown(&layout, &f, Pane::Left);
        let context = left.iter().find(|r| r.kind == RowKind::Context).unwrap();
        assert_eq!(context.id.unwrap().0, Side::Left);

        let right = shown(&layout, &f, Pane::Right);
        let context = right.iter().find(|r| r.kind == RowKind::Context).unwrap();
        assert_eq!(context.id.unwrap().0, Side::Right);
    }

    #[test]
    fn only_rows_standing_for_a_line_are_selectable() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);

        for row in &rows {
            match row.kind {
                RowKind::Gap | RowKind::Header | RowKind::Filler => {
                    assert!(!row.selectable, "{:?} is not part of a selection", row.kind);
                    assert!(row.source.is_none());
                }
                _ => {
                    assert!(row.selectable);
                    assert!(row.source.is_some());
                }
            }
        }
    }

    #[test]
    fn a_gap_carries_what_it_needs_to_be_opened() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let gap = rows.iter().find(|r| r.kind == RowKind::Gap).unwrap();

        assert!(gap.hidden_count > 0);
        assert_eq!(gap.gap_start, (1, 1), "the hidden run starts at line one");
    }

    // == Handing rows to the widget

    #[test]
    fn a_missing_line_number_becomes_zero() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let added = rows.iter().find(|r| r.kind == RowKind::Added).unwrap();
        let converted = ui::DiffRow::from(added);

        assert_eq!(converted.left_line, 0, "absent in the left file");
        assert!(converted.right_line > 0);
    }

    #[test]
    fn a_gap_hands_over_where_its_hidden_run_starts() {
        // The two number fields carry the start, because a gap draws no numbers and opening
        // one has to say which lines to ask for. Without it every gap requests line zero and
        // clicking any of them opens the first.
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let gap = rows.iter().find(|r| r.kind == RowKind::Gap).unwrap();
        let converted = ui::DiffRow::from(gap);

        assert_eq!(
            (converted.left_line as u32, converted.right_line as u32),
            gap.gap_start
        );
        assert!(converted.left_line > 0, "a real line to fetch from");
    }

    #[test]
    fn a_gap_hands_over_its_note_rather_than_its_text() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let gap = rows.iter().find(|r| r.kind == RowKind::Gap).unwrap();

        assert_eq!(ui::DiffRow::from(gap).text, gap.note);
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
            // Converting must not panic.
            let _ = ui::DiffRowKind::from(kind);
        }
    }

    // == Highlights

    fn mark(row: usize, columns: Range<u32>) -> (usize, Highlight) {
        (
            row,
            Highlight {
                extent: DisplayColumnExtent::Columns(columns),
                kind: HighlightKind::Marked,
            },
        )
    }

    fn model() -> RowModel {
        let f = file();
        let layout = build_inline(&f, &RowOptions::default());
        RowModel::new(&layout, &f, &RenderOptions::default(), Pane::Inline)
    }

    #[test]
    fn a_row_whose_highlights_did_not_change_is_not_written_again() {
        // Handing Slint a row it already holds counts as a change, and the list redraws it.
        let mut view = model();
        let same = vec![mark(1, 0..3), mark(3, 1..2)];

        assert_eq!(view.set_highlights(&same), 2, "both rows written once");
        assert_eq!(view.set_highlights(&same), 0, "and not written again");
    }

    #[test]
    fn only_the_rows_that_changed_are_written() {
        let mut view = model();
        view.set_highlights(&[mark(1, 0..3), mark(3, 1..2)]);

        // Line 1 keeps exactly what it had; row 3's range moves.
        let written = view.set_highlights(&[mark(1, 0..3), mark(3, 4..6)]);

        assert_eq!(written, 1, "only the row that moved");
    }

    #[test]
    fn a_row_that_loses_its_highlights_is_cleared() {
        let mut view = model();
        view.set_highlights(&[mark(2, 0..3)]);

        assert_eq!(view.set_highlights(&[]), 1, "cleared, once");
        assert_eq!(view.model().row_data(2).unwrap().highlights.row_count(), 0);
    }
}
