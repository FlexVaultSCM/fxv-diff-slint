//! The rows a pane is showing, and the handover to the widget.
//!
//! Holds each row in both the form this crate reasons about and the form Slint binds to, so a
//! highlight can change without either being rebuilt.
//!
//! Nothing here asks where the rows came from. A laid-out diff and a plain file listing arrive
//! by the same door.

// == Std crates
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

// == External Crates
use slint::{Model, ModelRc, VecModel};

// == Internal Crates
use crate::diff::layout::GapState;
use crate::span::Side;
use crate::ui;
use crate::view::row::{Channel, DisplayColumnExtent, DisplayedRow, Highlight, RowClass};

/// The rows a pane is showing, in both the form this crate reasons about and the form the
/// widget draws, kept together so highlights can change without either being rebuilt.
pub struct RowModel {
    rows: Vec<DisplayedRow>,
    model: Rc<VecModel<ui::DiffRow>>,
    longest_line_columns: i32,
    /// Which row shows each line, for resolving a span that names a file and a line number.
    ///
    /// Built once, because the rows do not change while the model exists: opening a gap builds
    /// a new one. The first row wins where a line appears twice, which an inline view does for
    /// an unchanged line.
    line_index: HashMap<(Side, u32), usize>,
    /// What each channel is currently drawing, per channel, in row order.
    ///
    /// Kept so a change can write back only the rows whose ranges actually differ. Handing
    /// Slint a row it already holds counts as a change to the model, so the list invalidates
    /// and redraws that row for nothing.
    ///
    /// Ordered by channel, which is the order they are painted in.
    channels: BTreeMap<Channel, Vec<(usize, Vec<DisplayColumnExtent>)>>,
}

impl RowModel {
    /// Builds a pane from rows supplied directly.
    ///
    /// The route for content that is not a diff. Everything a pane does past this point, the
    /// grid, highlights, selection and scrolling, works from these rows and asks nothing about
    /// where they came from, so a plain file listing is as good an input as a laid-out diff.
    pub fn from_rows(mut rows: Vec<DisplayedRow>) -> Self {
        mark_runs(&mut rows);

        let longest_line_columns = rows.iter().map(|r| r.columns).max().unwrap_or(0) as i32;
        let converted: Vec<ui::DiffRow> = rows.iter().map(|r| r.into()).collect();

        let mut line_index = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some(id) = row.id {
                line_index.entry(id).or_insert(index);
            }
        }

        RowModel {
            rows,
            model: Rc::new(VecModel::from(converted)),
            longest_line_columns,
            line_index,
            channels: BTreeMap::new(),
        }
    }

    /// Which row shows a line, if this pane is showing it at all.
    ///
    /// A pane of a split view holds one side, and a line inside a gap is not on screen, so a
    /// span naming either has no row here. That is ordinary rather than an error.
    pub fn row_of(&self, side: Side, line: u32) -> Option<usize> {
        self.line_index.get(&(side, line)).copied()
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

    /// Replaces everything one channel is painting, leaving every other channel alone.
    ///
    /// Returns how many rows were written, which is the measure of what this cost. A row is
    /// written only if what it draws actually changed, so resending an unchanged set costs
    /// nothing and a change to one channel does not touch rows that only another paints.
    pub fn set_channel(
        &mut self,
        channel: Channel,
        ranges: &[(usize, DisplayColumnExtent)],
    ) -> usize {
        let wanted = group_by_row(ranges, self.rows.len());
        let previous = self.channels.get(&channel).map_or(&[][..], Vec::as_slice);
        let touched = differing_rows(previous, &wanted);

        self.channels.insert(channel, wanted);
        touched
            .into_iter()
            .filter(|index| self.write(*index))
            .count()
    }

    /// Rebuilds one row's highlights from every channel and hands it back to Slint.
    ///
    /// Reports whether it did, since a row that has gone missing is not a write.
    fn write(&self, index: usize) -> bool {
        let Some(mut row) = self.model.row_data(index) else {
            return false;
        };
        // Ascending channel order, so a higher channel is drawn over a lower one.
        let mut all = Vec::new();
        for (channel, rows) in &self.channels {
            if let Ok(at) = rows.binary_search_by_key(&index, |(i, _)| *i) {
                all.extend(rows[at].1.iter().map(|extent| Highlight {
                    extent: extent.clone(),
                    channel: *channel,
                }));
            }
        }
        row.highlights = to_slint_highlights(&all);
        self.model.set_row_data(index, row);
        true
    }
}

/// Records which run of selectable rows each row belongs to.
///
/// A run is a maximal stretch of selectable rows. Gaps and headers are not selectable and so
/// separate one run from the next, which is what stops a selection crossing a gap: a drag
/// clamps to the run it began in, and the row it began on is the only thing that has to know
/// where that run ends.
fn mark_runs(rows: &mut [DisplayedRow]) {
    // One pass, closing a run whenever an unselectable row ends it and once more at the end of
    // the list. Written as a forward scan rather than a search-and-skip so that it advances on
    // every step whatever the rows contain.
    let mut start = 0;
    for at in 0..=rows.len() {
        if at < rows.len() && rows[at].selectable {
            continue;
        }
        for row in &mut rows[start..at] {
            row.selectable_run = start as u32..at as u32;
        }
        start = at + 1;
    }
}

/// Rows whose contents differ between two grouped lists, both in row order.
///
/// Walked in step rather than searched, since both are sorted and either can hold rows the
/// other does not.
fn differing_rows(
    previous: &[(usize, Vec<DisplayColumnExtent>)],
    wanted: &[(usize, Vec<DisplayColumnExtent>)],
) -> Vec<usize> {
    let mut touched = Vec::new();
    let (mut a, mut b) = (0, 0);
    while a < previous.len() || b < wanted.len() {
        match (previous.get(a), wanted.get(b)) {
            // Had ranges, has none now.
            (Some((i, _)), None) => {
                touched.push(*i);
                a += 1;
            }
            // Has ranges, had none.
            (None, Some((j, _))) => {
                touched.push(*j);
                b += 1;
            }
            (Some((i, old)), Some((j, new))) => {
                if i < j {
                    touched.push(*i);
                    a += 1;
                } else if j < i {
                    touched.push(*j);
                    b += 1;
                } else {
                    if old != new {
                        touched.push(*i);
                    }
                    a += 1;
                    b += 1;
                }
            }
            (None, None) => break,
        }
    }
    touched
}

/// Gathers one channel's ranges per row, in row order, dropping any that name a row that is
/// not there.
///
/// Sorted rather than grouped in place so this and the previous state can be walked in step,
/// instead of scanning one for every entry of the other.
fn group_by_row(
    ranges: &[(usize, DisplayColumnExtent)],
    rows: usize,
) -> Vec<(usize, Vec<DisplayColumnExtent>)> {
    let mut ordered: Vec<&(usize, DisplayColumnExtent)> =
        ranges.iter().filter(|(i, _)| *i < rows).collect();
    ordered.sort_by_key(|(i, _)| *i);

    let mut grouped: Vec<(usize, Vec<DisplayColumnExtent>)> = Vec::new();
    for (index, extent) in ordered {
        match grouped.last_mut() {
            Some((at, list)) if at == index => list.push(extent.clone()),
            _ => grouped.push((*index, vec![extent.clone()])),
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
        // A gap carries where its hidden run starts rather than a line of its own, and the
        // controls need it to say what to fetch.
        let (left, right) = if row.class == RowClass::GAP {
            row.gap_start
        } else {
            (row.numbers[0].unwrap_or(0), row.numbers[1].unwrap_or(0))
        };

        ui::DiffRow {
            class: row.class.0 as i32,
            numbered: row.numbered,
            full_width: row.full_width,
            // Zero means "no number in this column". Line numbers are 1-based, so it cannot
            // collide with a real one.
            left_line: left as i32,
            right_line: right as i32,
            text: row.text.clone(),
            note: row.note.clone(),
            id_side: row.id.map_or(ui::DiffSide::Both, |(side, _)| side.into()),
            // Zero means this row names no line. A row that does have one is 1-based.
            id_line: row.id.map_or(0, |(_, line)| line) as i32,
            columns: row.columns as i32,
            hidden_count: row.hidden_count as i32,
            gap_state: row.gap_state.into(),
            // Zero count means nothing is in flight. A fetch of no lines is not a thing.
            busy_start: row.pending.map_or(0, |(start, _)| start) as i32,
            busy_count: row.pending.map_or(0, |(_, count)| count) as i32,
            highlights: ModelRc::default(),
            // An empty run is how a row says it is not selectable at all.
            run_start: row.selectable_run.start as i32,
            run_end: row.selectable_run.end as i32,
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
                channel: h.channel.0 as i32,
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(converted)))
}

impl From<Side> for ui::DiffSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => ui::DiffSide::Left,
            Side::Right => ui::DiffSide::Right,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::layout::Layout;
    use crate::diff::layout::{build_inline, build_split, RowOptions};
    use crate::diff::render::{render_diff, Pane};
    use crate::span::Side;
    use crate::test_fixtures::{file, shown};
    use crate::text::RenderOptions;
    use std::ops::Range;

    fn kinds(rows: &[DisplayedRow]) -> Vec<RowClass> {
        rows.iter().map(|r| r.class).collect()
    }

    // == Rendering

    #[test]
    fn tabs_are_expanded_in_row_text() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let removed = rows.iter().find(|r| r.class == RowClass::REMOVED).unwrap();

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
        let wide = RowModel::from_rows(render_diff(
            &layout,
            &f,
            &RenderOptions {
                tab_width: 8,
                ..RenderOptions::default()
            },
            Pane::Inline,
        ));
        let removed = wide
            .rows()
            .iter()
            .find(|r| r.class == RowClass::REMOVED)
            .unwrap();

        assert!(removed.text.starts_with("        "), "eight columns");
    }

    #[test]
    fn the_longest_line_is_measured_after_expansion() {
        let f = file();
        let layout = build_inline(&f, &RowOptions::default());
        let opts = RenderOptions::default();
        let model = RowModel::from_rows(render_diff(&layout, &f, &opts, Pane::Inline));

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

        let left = RowModel::from_rows(render_diff(&layout, &f, &opts, Pane::Left));
        let right = RowModel::from_rows(render_diff(&layout, &f, &opts, Pane::Right));

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
            RowModel::from_rows(render_diff(
                &Layout::default(),
                &f,
                &RenderOptions::default(),
                Pane::Inline
            ))
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
            !kinds(&right).contains(&RowClass::FILLER),
            "an even change needs no padding"
        );
        assert!(kinds(&right).contains(&RowClass::ADDED));
    }

    #[test]
    fn an_inline_pane_numbers_both_columns_of_an_unchanged_line() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let context = rows.iter().find(|r| r.class == RowClass::CONTEXT).unwrap();

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
        let context = left.iter().find(|r| r.class == RowClass::CONTEXT).unwrap();
        assert_eq!(context.id.unwrap().0, Side::Left);

        let right = shown(&layout, &f, Pane::Right);
        let context = right.iter().find(|r| r.class == RowClass::CONTEXT).unwrap();
        assert_eq!(context.id.unwrap().0, Side::Right);
    }

    #[test]
    fn only_rows_standing_for_a_line_are_selectable() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);

        for row in &rows {
            match row.class {
                RowClass::GAP | RowClass::HEADER | RowClass::FILLER => {
                    assert!(
                        !row.selectable,
                        "{:?} is not part of a selection",
                        row.class
                    );
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
        let gap = rows.iter().find(|r| r.class == RowClass::GAP).unwrap();

        assert!(gap.hidden_count > 0);
        assert_eq!(gap.gap_start, (1, 1), "the hidden run starts at line one");
    }

    // == Handing rows to the widget

    #[test]
    fn a_missing_line_number_becomes_zero() {
        let f = file();
        let rows = shown(&build_inline(&f, &RowOptions::default()), &f, Pane::Inline);
        let added = rows.iter().find(|r| r.class == RowClass::ADDED).unwrap();
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
        let gap = rows.iter().find(|r| r.class == RowClass::GAP).unwrap();
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
        let gap = rows.iter().find(|r| r.class == RowClass::GAP).unwrap();

        assert_eq!(ui::DiffRow::from(gap).text, gap.note);
    }

    // == Highlights

    fn mark(row: usize, columns: Range<u32>) -> (usize, DisplayColumnExtent) {
        (row, DisplayColumnExtent::Columns(columns))
    }

    fn model() -> RowModel {
        let f = file();
        let layout = build_inline(&f, &RowOptions::default());
        RowModel::from_rows(render_diff(
            &layout,
            &f,
            &RenderOptions::default(),
            Pane::Inline,
        ))
    }

    #[test]
    fn a_row_whose_highlights_did_not_change_is_not_written_again() {
        // Handing Slint a row it already holds counts as a change, and the list redraws it.
        let mut view = model();
        let same = vec![mark(1, 0..3), mark(3, 1..2)];

        assert_eq!(
            view.set_channel(Channel::MARKED, &same),
            2,
            "both rows once"
        );
        assert_eq!(view.set_channel(Channel::MARKED, &same), 0, "not again");
    }

    #[test]
    fn only_the_rows_that_changed_are_written() {
        let mut view = model();
        view.set_channel(Channel::MARKED, &[mark(1, 0..3), mark(3, 1..2)]);

        // Row 1 keeps exactly what it had; row 3's range moves.
        let written = view.set_channel(Channel::MARKED, &[mark(1, 0..3), mark(3, 4..6)]);

        assert_eq!(written, 1, "only the row that moved");
    }

    #[test]
    fn a_row_that_loses_its_highlights_is_cleared() {
        let mut view = model();
        view.set_channel(Channel::MARKED, &[mark(2, 0..3)]);

        assert_eq!(view.set_channel(Channel::MARKED, &[]), 1, "cleared, once");
        assert_eq!(view.model().row_data(2).unwrap().highlights.row_count(), 0);
    }

    #[test]
    fn setting_one_channel_leaves_another_channels_rows_alone() {
        // The reason a channel is set on its own: a host repainting its marks must not cost
        // anything on the rows that only carry a selection.
        let mut view = model();
        view.set_channel(Channel::SELECTION, &[mark(1, 0..3)]);
        view.set_channel(Channel::MARKED, &[mark(3, 0..3)]);

        let written = view.set_channel(Channel::MARKED, &[mark(3, 4..6)]);

        assert_eq!(written, 1, "row 3 only; row 1 carries a different channel");
    }

    #[test]
    fn a_row_carrying_two_channels_draws_both() {
        let mut view = model();
        view.set_channel(Channel::SELECTION, &[mark(1, 0..3)]);
        view.set_channel(Channel::MARKED, &[mark(1, 5..7)]);

        let drawn = view.model().row_data(1).unwrap().highlights;
        assert_eq!(drawn.row_count(), 2, "one range from each channel");
        // Ascending channel order, so a higher channel paints over a lower one.
        assert_eq!(
            drawn.row_data(0).unwrap().channel,
            Channel::SELECTION.0 as i32
        );
        assert_eq!(drawn.row_data(1).unwrap().channel, Channel::MARKED.0 as i32);
    }

    #[test]
    fn clearing_one_channel_leaves_the_other_drawn() {
        let mut view = model();
        view.set_channel(Channel::SELECTION, &[mark(1, 0..3)]);
        view.set_channel(Channel::MARKED, &[mark(1, 5..7)]);

        view.set_channel(Channel::SELECTION, &[]);

        let drawn = view.model().row_data(1).unwrap().highlights;
        assert_eq!(
            drawn.row_count(),
            1,
            "the mark survives the selection going"
        );
        assert_eq!(drawn.row_data(0).unwrap().channel, Channel::MARKED.0 as i32);
    }

    #[test]
    fn a_line_resolves_to_the_row_showing_it() {
        let view = model();
        let row = view
            .rows()
            .iter()
            .position(|r| r.id.is_some())
            .expect("the fixture shows some lines");
        let (side, line) = view.rows()[row].id.unwrap();

        assert_eq!(view.row_of(side, line), Some(row));
    }
}
