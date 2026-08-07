//! Flattening a parsed diff into the rows a view renders.
//!
//! A unified diff describes changes; a view draws lines. This is where one becomes the other.
//! It is plain data in and plain data out, with no Slint involved, because the interesting
//! parts (aligning two sides, working out what is hidden between hunks) are worth testing
//! without a window in the way.

// == Std
use std::ops::Range;

// == External Crates
use slint::SharedString;

// == Internal Crates
use crate::model::{DiffLine, Fetch, FetchState, FileDiff, Hunk, LineKind, LineRef};
use crate::span::Side;
use crate::text::{display_width, render_line, RenderOptions};

/// Why a gap row's lines are not on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GapState {
    /// Nobody has asked for them.
    #[default]
    Hidden,
    /// Somebody has, and they have not arrived.
    Waiting,
    /// Somebody did, and it did not work. The row's text says why.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Names the file, and carries its rename or mode change if it has one.
    Header,
    /// Unchanged content, shown to give the change context.
    Context,
    Added,
    Removed,
    /// Content that exists but is not shown. Expanding it replaces the row with the lines.
    Gap,
    /// Nothing on this side. Keeps the two panes of a side-by-side view in step.
    Filler,
}

/// One row of the rendered diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    /// Line number in the left file. Absent for added lines and fillers.
    pub left_line: Option<u32>,
    /// Line number in the right file. Absent for removed lines and fillers.
    pub right_line: Option<u32>,
    /// Display text, tabs already expanded. Empty for fillers.
    ///
    /// Shared rather than owned outright. A side-by-side view puts the same context line in
    /// both panes, and every row is handed to the widget as well, so an owned string would be
    /// copied three times over for text that never changes after it is built.
    pub text: SharedString,
    /// How many lines a gap row is hiding. Zero for every other kind.
    pub hidden_count: u32,
    /// Only meaningful on a gap row.
    pub gap_state: GapState,
    /// Columns the text occupies.
    ///
    /// Recorded when the row is built because rendering has just walked the line and knows
    /// the answer. Measuring the finished rows instead would mean a second pass over every
    /// character in the diff to recover a number that was already in hand.
    pub columns: u32,
    /// The line this row was rendered from, for anything that needs the original text rather
    /// than what is drawn. Absent for gaps, headers and fillers, which stand for no line.
    pub source: Option<LineRef>,
}

impl Row {
    /// Which file names this row, and the line number it has there.
    ///
    /// A removed line exists only on the left and an added line only on the right, so each
    /// decides itself. An unchanged line exists on both, and nothing about the line says which
    /// is meant: an inline view means the right, being the file as it stands after the change,
    /// while a pane of a side-by-side view means its own side, since a selection made in the
    /// left pane is about the left file whatever the line is. That is what `context_side`
    /// supplies.
    ///
    /// Rows standing for no line have no number on either side and return `None`.
    pub fn file_line(&self, context_side: Side) -> Option<(Side, u32)> {
        let side = match self.kind {
            RowKind::Removed => Side::Left,
            RowKind::Added => Side::Right,
            _ => context_side,
        };
        let line = match side {
            Side::Left => self.left_line,
            Side::Right => self.right_line,
        }?;
        Some((side, line))
    }

    fn filler() -> Self {
        Row {
            kind: RowKind::Filler,
            left_line: None,
            right_line: None,
            text: SharedString::new(),
            hidden_count: 0,
            gap_state: GapState::Hidden,
            columns: 0,
            source: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RowOptions {
    /// How each line is turned into the text a view draws.
    pub render: RenderOptions,
    /// Prepend a row naming the file.
    pub include_file_header: bool,
    /// Total lines in the left file. Without it there is no way to know whether anything
    /// follows the last hunk, so no trailing gap is produced.
    pub left_total_lines: Option<u32>,
    /// Total lines in the right file. See `left_total_lines`.
    pub right_total_lines: Option<u32>,
}

/// Rows plus the measurement a view needs to size its horizontal scroll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rows {
    pub rows: Vec<Row>,
    /// Columns occupied by the longest row's text.
    pub longest_line_columns: u32,
}

impl Rows {
    fn from_rows(rows: Vec<Row>) -> Self {
        let longest_line_columns = rows.iter().map(|r| r.columns).max().unwrap_or(0);
        Rows {
            rows,
            longest_line_columns,
        }
    }
}

/// The two panes of a side-by-side view.
///
/// Both sides always have the same number of rows, so a given index names the same place in
/// the diff on both, which is what lets the panes scroll together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideBySideRows {
    pub left: Rows,
    pub right: Rows,
}

impl SideBySideRows {
    /// The column count both panes should be sized by.
    ///
    /// It is the wider of the two on purpose. The panes scroll horizontally together, so
    /// giving each its own width would make their scroll ranges differ and let the sides drift
    /// out of step as the view moves sideways.
    pub fn longest_line_columns(&self) -> u32 {
        self.left
            .longest_line_columns
            .max(self.right.longest_line_columns)
    }
}

/// Builds the rows for a single-column view showing removals above additions.
pub fn build_inline(file: &FileDiff, opts: &RowOptions) -> Rows {
    let mut rows = Vec::new();

    if opts.include_file_header {
        rows.push(header_row(file));
    }

    let mut walker = GapWalker::new(file, opts);
    for (h, hunk) in file.hunks().iter().enumerate() {
        rows.extend(walker.gap_before(hunk));
        for block in change_blocks(&hunk.lines) {
            match block {
                Block::Context(i) => rows.push(content_row(h, i, &hunk.lines, opts)),
                Block::Change { removed, added } => {
                    rows.extend(removed.map(|i| content_row(h, i, &hunk.lines, opts)));
                    rows.extend(added.map(|i| content_row(h, i, &hunk.lines, opts)));
                }
            }
        }
    }
    rows.extend(walker.trailing_gap());

    Rows::from_rows(rows)
}

/// Builds the rows for a two-column view showing the files next to each other.
pub fn build_side_by_side(file: &FileDiff, opts: &RowOptions) -> SideBySideRows {
    let mut left = Vec::new();
    let mut right = Vec::new();

    if opts.include_file_header {
        left.push(header_row(file));
        right.push(header_row(file));
    }

    let mut walker = GapWalker::new(file, opts);
    for (h, hunk) in file.hunks().iter().enumerate() {
        for row in walker.gap_before(hunk) {
            left.push(row.clone());
            right.push(row);
        }
        pair_hunk(h, hunk, opts, &mut left, &mut right);
    }
    for row in walker.trailing_gap() {
        left.push(row.clone());
        right.push(row);
    }

    debug_assert_eq!(left.len(), right.len(), "panes must stay in step");

    SideBySideRows {
        left: Rows::from_rows(left),
        right: Rows::from_rows(right),
    }
}

/// Lays a hunk's lines out in two columns.
///
/// The two runs of a change are placed opposite each other and the shorter is padded, which is
/// what keeps a three-line removal facing the five-line addition that replaced it.
fn pair_hunk(
    hunk_index: usize,
    hunk: &Hunk,
    opts: &RowOptions,
    left: &mut Vec<Row>,
    right: &mut Vec<Row>,
) {
    for block in change_blocks(&hunk.lines) {
        match block {
            Block::Context(i) => {
                // The same line goes in both panes. Cloning shares the text rather than
                // copying it, which matters because context is most of a diff.
                let row = content_row(hunk_index, i, &hunk.lines, opts);
                left.push(row.clone());
                right.push(row);
            }
            Block::Change { removed, added } => {
                for slot in 0..removed.len().max(added.len()) {
                    let l = removed.start.checked_add(slot).filter(|i| *i < removed.end);
                    let r = added.start.checked_add(slot).filter(|i| *i < added.end);
                    left.push(row_or_filler(hunk_index, l, &hunk.lines, opts));
                    right.push(row_or_filler(hunk_index, r, &hunk.lines, opts));
                }
            }
        }
    }
}

/// A hunk's lines grouped into what they mean rather than the order they were written in.
///
/// Both layouts walk this, and they differ only in what they do with a `Change`: inline puts
/// the two runs one after the other, side by side puts them opposite each other. Word-level
/// highlighting will want the same grouping, since it has to pair a removed line with the
/// added line that replaced it.
enum Block {
    Context(usize),
    /// Lines taken out, and the lines put in their place. Either may be empty: a pure
    /// insertion has no removals and a pure deletion has no additions.
    ///
    /// Held as index ranges rather than slices so a row can record which line it came from.
    Change {
        removed: Range<usize>,
        added: Range<usize>,
    },
}

fn change_blocks(lines: &[DiffLine]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].kind == LineKind::Context {
            blocks.push(Block::Context(i));
            i += 1;
            continue;
        }

        // Removals then additions, the order a unified diff writes them. Taking each run
        // independently means the reverse order costs nothing.
        let removed_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let removed = removed_start..i;

        let added_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }

        blocks.push(Block::Change {
            removed,
            added: added_start..i,
        });
    }

    blocks
}

/// Turns a parsed line into the row that renders it.
///
/// The line numbers come across unchanged rather than being cleared per side: a removed line
/// already has no right-hand number and an added line has no left-hand one.
fn content_row(hunk: usize, index: usize, hunk_lines: &[DiffLine], opts: &RowOptions) -> Row {
    let line = &hunk_lines[index];
    let (text, columns) = render_line(&line.text, line.line_ending, &opts.render);
    Row {
        kind: match line.kind {
            LineKind::Context => RowKind::Context,
            LineKind::Added => RowKind::Added,
            LineKind::Removed => RowKind::Removed,
        },
        left_line: line.left_line,
        right_line: line.right_line,
        text: text.as_str().into(),
        hidden_count: 0,
        gap_state: GapState::Hidden,
        columns: columns as u32,
        source: Some(LineRef {
            hunk: hunk as u32,
            line: index as u32,
        }),
    }
}

/// A row for the line at that index, or blank space where that side of a change has run out.
fn row_or_filler(
    hunk: usize,
    index: Option<usize>,
    hunk_lines: &[DiffLine],
    opts: &RowOptions,
) -> Row {
    match index {
        Some(index) => content_row(hunk, index, hunk_lines, opts),
        None => Row::filler(),
    }
}

fn header_row(file: &FileDiff) -> Row {
    Row {
        kind: RowKind::Header,
        left_line: None,
        right_line: None,
        columns: display_width(file.display_path()) as u32,
        text: file.display_path().into(),
        hidden_count: 0,
        gap_state: GapState::Hidden,
        source: None,
    }
}

/// Tracks how far through each file the hunks have reached, so the lines between them can be
/// reported as gaps.
struct GapWalker<'a> {
    /// First line not yet covered, on each side. Both start at 1.
    left_next: u32,
    right_next: u32,
    left_total: Option<u32>,
    right_total: Option<u32>,
    /// Set once any hunk has been seen, so a file with no hunks produces no trailing gap.
    seen_hunk: bool,
    fetches: &'a [Fetch],
}

impl<'a> GapWalker<'a> {
    fn new(file: &'a FileDiff, opts: &RowOptions) -> Self {
        GapWalker {
            left_next: 1,
            right_next: 1,
            left_total: opts.left_total_lines,
            right_total: opts.right_total_lines,
            seen_hunk: false,
            fetches: &file.fetches,
        }
    }

    fn gap_before(&mut self, hunk: &Hunk) -> Vec<Row> {
        // Taken as the larger of the two sides. They agree wherever both files have the
        // content, and where one does not (a file that was added, so every left number is
        // zero) the other still gives the right answer.
        let left_hidden = hunk.left_start.saturating_sub(self.left_next);
        let right_hidden = hunk.right_start.saturating_sub(self.right_next);
        let hidden = left_hidden.max(right_hidden);

        let rows = self.gap_rows(hidden, hunk.heading.as_deref());

        self.left_next = (hunk.left_start + hunk.left_len).max(1);
        self.right_next = (hunk.right_start + hunk.right_len).max(1);
        self.seen_hunk = true;
        rows
    }

    fn trailing_gap(&self) -> Vec<Row> {
        if !self.seen_hunk {
            return Vec::new();
        }

        let left_hidden = self
            .left_total
            .map_or(0, |total| (total + 1).saturating_sub(self.left_next));
        let right_hidden = self
            .right_total
            .map_or(0, |total| (total + 1).saturating_sub(self.right_next));
        let hidden = left_hidden.max(right_hidden);

        self.gap_rows(hidden, None)
    }

    /// Splits a run of hidden lines around anything being fetched, so a gap being opened
    /// shows that rather than pretending nothing is happening.
    fn gap_rows(&self, hidden: u32, heading: Option<&str>) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut offset = 0;

        while offset < hidden {
            let fetch = self.fetch_at(self.right_next + offset);
            let mut run = 1;
            while offset + run < hidden && self.fetch_at(self.right_next + offset + run) == fetch {
                run += 1;
            }

            // The heading describes the hunk that follows, so it belongs to the stretch that
            // runs up to it and not to any earlier one.
            let heading = (offset + run == hidden).then_some(heading).flatten();
            rows.push(self.gap_row(offset, run, heading, fetch));
            offset += run;
        }

        rows
    }

    fn fetch_at(&self, right_line: u32) -> Option<&'a Fetch> {
        self.fetches
            .iter()
            .find(|f| right_line >= f.right_start && right_line < f.right_start + f.count)
    }

    fn gap_row(
        &self,
        offset: u32,
        count: u32,
        heading: Option<&str>,
        fetch: Option<&Fetch>,
    ) -> Row {
        let (state, text) = match fetch.map(|f| &f.state) {
            None => (GapState::Hidden, heading.unwrap_or_default()),
            Some(FetchState::Waiting) => (GapState::Waiting, ""),
            Some(FetchState::Failed(why)) => (GapState::Failed, why.as_str()),
        };

        Row {
            kind: RowKind::Gap,
            // Where the hidden run starts, so a host asked to open it knows what to fetch.
            left_line: Some(self.left_next + offset),
            right_line: Some(self.right_next + offset),
            columns: display_width(text) as u32,
            text: text.into(),
            hidden_count: count,
            gap_state: state,
            source: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffLine, FileChange, FileContent, LineEnding, LineOrigin};

    fn line(kind: LineKind, left: Option<u32>, right: Option<u32>, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.to_owned(),
            left_line: left,
            right_line: right,
            line_ending: LineEnding::Lf,
            origin: LineOrigin::Diff,
        }
    }

    fn file(hunks: Vec<Hunk>) -> FileDiff {
        FileDiff {
            left_path: Some("f.rs".into()),
            right_path: Some("f.rs".into()),
            change: FileChange::Modified,
            left_mode: None,
            right_mode: None,
            content: FileContent::Text { hunks },
            fetches: Vec::new(),
        }
    }

    /// A hunk replacing `removed` lines with `added` lines, wrapped in one line of context on
    /// each side, starting at `start` on both sides.
    ///
    /// It therefore covers `removed.len() + 2` lines on the left and `added.len() + 2` on the
    /// right, so `change_hunk(1, &["a"], &["b"])` occupies lines 1 to 3 and the next free line
    /// is 4. Getting that count wrong is the easiest way to write a gap test that fails for
    /// the wrong reason.
    fn change_hunk(start: u32, removed: &[&str], added: &[&str]) -> Hunk {
        let mut lines = vec![line(
            LineKind::Context,
            Some(start),
            Some(start),
            "context before",
        )];
        for (i, text) in removed.iter().enumerate() {
            lines.push(line(
                LineKind::Removed,
                Some(start + 1 + i as u32),
                None,
                text,
            ));
        }
        for (i, text) in added.iter().enumerate() {
            lines.push(line(
                LineKind::Added,
                None,
                Some(start + 1 + i as u32),
                text,
            ));
        }
        let left_len = 2 + removed.len() as u32;
        let right_len = 2 + added.len() as u32;
        lines.push(line(
            LineKind::Context,
            Some(start + 1 + removed.len() as u32),
            Some(start + 1 + added.len() as u32),
            "context after",
        ));
        Hunk {
            left_start: start,
            left_len,
            right_start: start,
            right_len,
            heading: None,
            lines,
        }
    }

    fn kinds(rows: &[Row]) -> Vec<RowKind> {
        rows.iter().map(|r| r.kind).collect()
    }

    // == Inline

    #[test]
    fn inline_keeps_diff_order_with_removals_above_additions() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(
            kinds(&rows.rows),
            vec![
                RowKind::Context,
                RowKind::Removed,
                RowKind::Added,
                RowKind::Context
            ]
        );
    }

    #[test]
    fn inline_carries_the_line_numbers_through() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let rows = build_inline(&f, &RowOptions::default());
        let pairs: Vec<_> = rows
            .rows
            .iter()
            .map(|r| (r.left_line, r.right_line))
            .collect();
        assert_eq!(
            pairs,
            vec![
                (Some(1), Some(1)),
                (Some(2), None),
                (None, Some(2)),
                (Some(3), Some(3)),
            ]
        );
    }

    #[test]
    fn a_file_header_is_opt_in() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);

        let without = build_inline(&f, &RowOptions::default());
        assert_ne!(without.rows[0].kind, RowKind::Header);

        let with = build_inline(
            &f,
            &RowOptions {
                include_file_header: true,
                ..Default::default()
            },
        );
        assert_eq!(with.rows[0].kind, RowKind::Header);
        assert_eq!(with.rows[0].text, "f.rs");
    }

    // == Gaps

    #[test]
    fn a_hunk_that_does_not_start_at_line_one_is_preceded_by_a_gap() {
        let f = file(vec![change_hunk(10, &["old"], &["new"])]);
        let rows = build_inline(&f, &RowOptions::default());

        assert_eq!(rows.rows[0].kind, RowKind::Gap);
        // Lines 1 to 9 are hidden.
        assert_eq!(rows.rows[0].hidden_count, 9);
        assert_eq!(rows.rows[0].left_line, Some(1));
        assert_eq!(rows.rows[0].right_line, Some(1));
    }

    #[test]
    fn a_hunk_starting_at_line_one_has_no_gap_before_it() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let rows = build_inline(&f, &RowOptions::default());
        assert_ne!(rows.rows[0].kind, RowKind::Gap);
    }

    #[test]
    fn the_lines_between_two_hunks_become_a_gap() {
        // The first hunk covers 1 to 3, the second starts at 20, so 4 to 19 are hidden.
        let f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(20, &["c"], &["d"]),
        ]);
        let rows = build_inline(&f, &RowOptions::default());

        let gaps: Vec<&Row> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .collect();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].hidden_count, 16);
        assert_eq!(gaps[0].left_line, Some(4));
    }

    #[test]
    fn adjacent_hunks_produce_no_gap() {
        // The first hunk covers 1 to 3; the second starts immediately after.
        let f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(4, &["c"], &["d"]),
        ]);
        let rows = build_inline(&f, &RowOptions::default());
        assert!(!rows.rows.iter().any(|r| r.kind == RowKind::Gap));
    }

    #[test]
    fn a_trailing_gap_needs_the_file_length_to_be_known() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);

        // Without totals there is no way to tell whether anything follows the last hunk.
        let unknown = build_inline(&f, &RowOptions::default());
        assert!(!unknown.rows.iter().any(|r| r.kind == RowKind::Gap));

        // The hunk covers 1 to 3, so lines 4 to 30 remain.
        let known = build_inline(
            &f,
            &RowOptions {
                left_total_lines: Some(30),
                right_total_lines: Some(30),
                ..Default::default()
            },
        );
        let last = known.rows.last().unwrap();
        assert_eq!(last.kind, RowKind::Gap);
        assert_eq!(last.hidden_count, 27);
        assert_eq!(last.left_line, Some(4));
    }

    #[test]
    fn a_hunk_reaching_the_end_of_the_file_has_no_trailing_gap() {
        // The hunk covers every line the file has.
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        let rows = build_inline(
            &f,
            &RowOptions {
                left_total_lines: Some(3),
                right_total_lines: Some(3),
                ..Default::default()
            },
        );
        assert!(!rows.rows.iter().any(|r| r.kind == RowKind::Gap));
    }

    /// A file whose first hunk starts at line 10, so lines 1 to 9 are hidden.
    fn file_with_leading_gap() -> FileDiff {
        file(vec![change_hunk(10, &["old"], &["new"])])
    }

    #[test]
    fn supplied_lines_replace_the_part_of_a_gap_they_cover() {
        let mut f = file_with_leading_gap();
        // Open the first three of the nine hidden lines.
        f.expand(
            1,
            1,
            ["one", "two", "three"].iter().map(|s| (*s).to_owned()),
        );

        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(
            kinds(&rows.rows)[..5],
            [
                RowKind::Context,
                RowKind::Context,
                RowKind::Context,
                RowKind::Gap,
                RowKind::Context,
            ]
        );
        assert_eq!(rows.rows[0].text, "one");
        assert_eq!(rows.rows[2].text, "three");

        // What is left keeps its own numbering and count.
        let gap = &rows.rows[3];
        assert_eq!(gap.hidden_count, 6, "nine hidden less the three opened");
        assert_eq!(gap.left_line, Some(4));
        assert_eq!(gap.right_line, Some(4));
    }

    #[test]
    fn a_gap_can_be_opened_from_the_far_end() {
        let mut f = file_with_leading_gap();
        // The last three of lines 1 to 9.
        f.expand(
            7,
            7,
            ["seven", "eight", "nine"].iter().map(|s| (*s).to_owned()),
        );

        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(rows.rows[0].kind, RowKind::Gap);
        assert_eq!(rows.rows[0].hidden_count, 6);
        assert_eq!(rows.rows[0].left_line, Some(1));
        assert_eq!(rows.rows[1].text, "seven");
        assert_eq!(rows.rows[3].text, "nine");
    }

    #[test]
    fn opening_the_middle_of_a_gap_leaves_one_on_each_side() {
        let mut f = file_with_leading_gap();
        f.expand(4, 4, ["four", "five"].iter().map(|s| (*s).to_owned()));

        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(
            kinds(&rows.rows)[..4],
            [
                RowKind::Gap,
                RowKind::Context,
                RowKind::Context,
                RowKind::Gap
            ]
        );
        assert_eq!(rows.rows[0].hidden_count, 3, "lines 1 to 3");
        assert_eq!(rows.rows[3].hidden_count, 4, "lines 6 to 9");
        assert_eq!(rows.rows[3].left_line, Some(6));
    }

    #[test]
    fn a_fully_opened_gap_leaves_none() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, (1..=9).map(|n| format!("line {n}")));

        let rows = build_inline(&f, &RowOptions::default());
        assert!(!rows.rows.iter().any(|r| r.kind == RowKind::Gap));
        assert_eq!(rows.rows[0].text, "line 1");
        assert_eq!(rows.rows[8].text, "line 9");
    }

    #[test]
    fn opened_lines_are_rendered_like_any_other() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, ["\tindented".to_owned()]);

        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(rows.rows[0].text, "    indented", "tabs expanded");
        assert_eq!(rows.rows[0].columns, 12);
    }

    #[test]
    fn opening_a_gap_can_widen_the_longest_line() {
        let mut f = file_with_leading_gap();
        let before = build_inline(&f, &RowOptions::default()).longest_line_columns;

        f.expand(1, 1, ["x".repeat(200)]);
        let after = build_inline(&f, &RowOptions::default()).longest_line_columns;

        assert_eq!(after, 200, "the opened line is now the longest");
        assert!(after > before);
    }

    #[test]
    fn the_heading_stays_on_the_stretch_nearest_its_hunk() {
        let mut hunk = change_hunk(10, &["old"], &["new"]);
        hunk.heading = Some("impl Store {".into());
        let mut f = file(vec![hunk]);
        // Open the top of the gap, so a hidden stretch still sits against the hunk.
        f.expand(1, 1, ["one".to_owned()]);

        let rows = build_inline(&f, &RowOptions::default());
        let gaps: Vec<&Row> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .collect();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].text, "impl Store {");
    }

    #[test]
    fn side_by_side_opens_a_gap_on_both_panes_together() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);

        let sbs = build_side_by_side(&f, &RowOptions::default());
        assert_eq!(sbs.left.rows.len(), sbs.right.rows.len());
        assert_eq!(sbs.left.rows[0].text, "one");
        assert_eq!(sbs.right.rows[0].text, "one");
        assert_eq!(sbs.left.rows[2].kind, RowKind::Gap);
        assert_eq!(sbs.right.rows[2].kind, RowKind::Gap);
    }

    #[test]
    fn lines_far_from_any_hunk_become_a_hunk_of_their_own() {
        let mut f = file_with_leading_gap();
        // Line 40 is nowhere near the hidden range, and nothing would normally ask for it.
        // Adding it anyway is honest: the caller said to show that line, so it is shown, with
        // the distance to it left as a gap like any other.
        f.expand(40, 40, ["stray".to_owned()]);

        let rows = build_inline(&f, &RowOptions::default());
        assert!(rows.rows.iter().any(|r| r.text == "stray"));

        let gaps: Vec<u32> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .map(|r| r.hidden_count)
            .collect();
        assert_eq!(
            gaps,
            vec![9, 27],
            "before the first hunk, and before line 40"
        );
    }

    #[test]
    fn asking_twice_for_the_same_lines_changes_nothing() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);
        let once = build_inline(&f, &RowOptions::default());

        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);
        let twice = build_inline(&f, &RowOptions::default());

        assert_eq!(once.rows, twice.rows);
    }

    #[test]
    fn opening_the_whole_gap_merges_the_hunks_it_separated() {
        // Two hunks with sixteen hidden lines between them.
        let mut f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(20, &["c"], &["d"]),
        ]);
        assert_eq!(f.hunks().len(), 2);

        f.expand(4, 4, (4..20).map(|n| format!("line {n}")));

        assert_eq!(f.hunks().len(), 1, "nothing separates them any more");
        let rows = build_inline(&f, &RowOptions::default());
        assert!(!rows.rows.iter().any(|r| r.kind == RowKind::Gap));
    }

    #[test]
    fn a_gap_shows_the_heading_of_the_hunk_it_precedes() {
        let mut hunk = change_hunk(10, &["old"], &["new"]);
        hunk.heading = Some("impl Store {".into());
        let rows = build_inline(&file(vec![hunk]), &RowOptions::default());
        assert_eq!(rows.rows[0].text, "impl Store {");
    }

    #[test]
    fn a_file_with_no_hunks_produces_no_rows_and_no_gap() {
        let f = file(vec![]);
        let rows = build_inline(
            &f,
            &RowOptions {
                left_total_lines: Some(100),
                ..Default::default()
            },
        );
        assert!(rows.rows.is_empty(), "got {:?}", rows.rows);
    }

    #[test]
    fn an_added_file_has_no_leading_gap_despite_its_zero_left_numbers() {
        // Git writes `@@ -0,0 +1,2 @@` for a file that did not exist before.
        let hunk = Hunk {
            left_start: 0,
            left_len: 0,
            right_start: 1,
            right_len: 2,
            heading: None,
            lines: vec![
                line(LineKind::Added, None, Some(1), "one"),
                line(LineKind::Added, None, Some(2), "two"),
            ],
        };
        let rows = build_inline(&file(vec![hunk]), &RowOptions::default());
        assert_eq!(kinds(&rows.rows), vec![RowKind::Added, RowKind::Added]);
    }

    // == Side by side

    #[test]
    fn side_by_side_panes_always_have_the_same_length() {
        let f = file(vec![change_hunk(1, &["a", "b", "c"], &["x"])]);
        let sbs = build_side_by_side(&f, &RowOptions::default());
        assert_eq!(sbs.left.rows.len(), sbs.right.rows.len());
    }

    #[test]
    fn side_by_side_puts_context_on_both_sides() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let sbs = build_side_by_side(&f, &RowOptions::default());
        assert_eq!(sbs.left.rows[0].kind, RowKind::Context);
        assert_eq!(sbs.right.rows[0].kind, RowKind::Context);
        assert_eq!(sbs.left.rows[0].text, sbs.right.rows[0].text);
    }

    #[test]
    fn side_by_side_places_a_replacement_opposite_what_it_replaced() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let sbs = build_side_by_side(&f, &RowOptions::default());

        assert_eq!(sbs.left.rows[1].kind, RowKind::Removed);
        assert_eq!(sbs.left.rows[1].text, "old");
        assert_eq!(sbs.right.rows[1].kind, RowKind::Added);
        assert_eq!(sbs.right.rows[1].text, "new");
    }

    #[test]
    fn side_by_side_pads_the_shorter_side_of_an_uneven_change() {
        // Three lines replaced by five: the left side needs two fillers.
        let f = file(vec![change_hunk(
            1,
            &["a", "b", "c"],
            &["v", "w", "x", "y", "z"],
        )]);
        let sbs = build_side_by_side(&f, &RowOptions::default());

        assert_eq!(
            kinds(&sbs.left.rows),
            vec![
                RowKind::Context,
                RowKind::Removed,
                RowKind::Removed,
                RowKind::Removed,
                RowKind::Filler,
                RowKind::Filler,
                RowKind::Context,
            ]
        );
        assert_eq!(
            kinds(&sbs.right.rows),
            vec![
                RowKind::Context,
                RowKind::Added,
                RowKind::Added,
                RowKind::Added,
                RowKind::Added,
                RowKind::Added,
                RowKind::Context,
            ]
        );
    }

    #[test]
    fn a_pure_deletion_faces_fillers() {
        let f = file(vec![change_hunk(1, &["a", "b"], &[])]);
        let sbs = build_side_by_side(&f, &RowOptions::default());
        assert_eq!(
            kinds(&sbs.right.rows),
            vec![
                RowKind::Context,
                RowKind::Filler,
                RowKind::Filler,
                RowKind::Context
            ]
        );
        assert!(sbs.right.rows[1].text.is_empty());
        assert_eq!(sbs.right.rows[1].left_line, None);
        assert_eq!(sbs.right.rows[1].right_line, None);
    }

    #[test]
    fn side_by_side_repeats_a_gap_on_both_sides_to_stay_aligned() {
        let f = file(vec![change_hunk(10, &["old"], &["new"])]);
        let sbs = build_side_by_side(&f, &RowOptions::default());
        assert_eq!(sbs.left.rows[0].kind, RowKind::Gap);
        assert_eq!(sbs.right.rows[0].kind, RowKind::Gap);
        assert_eq!(
            sbs.left.rows[0].hidden_count,
            sbs.right.rows[0].hidden_count
        );
    }

    // == Text handling

    #[test]
    fn a_row_can_find_the_line_it_was_rendered_from() {
        let f = file(vec![change_hunk(1, &["\tremoved"], &["\tadded"])]);
        let rows = build_inline(&f, &RowOptions::default());

        for row in &rows.rows {
            let Some(at) = row.source else {
                continue;
            };
            let line = f.line(at).expect("source line should resolve");

            // The row carries display text; the line carries what the file holds. For a
            // tab-indented line those differ, which is the whole reason the reference exists.
            assert_eq!(row.left_line, line.left_line);
            assert_eq!(row.right_line, line.right_line);
            if line.text.contains('\t') {
                assert_ne!(row.text, line.text, "display text should not be the source");
                assert!(line.text.starts_with('\t'), "the source keeps its tab");
                assert!(row.text.starts_with("    "), "the row shows it expanded");
            }
        }
    }

    #[test]
    fn rows_that_stand_for_no_line_have_no_source() {
        let f = file(vec![change_hunk(10, &["a", "b"], &[])]);
        let opts = RowOptions {
            include_file_header: true,
            ..Default::default()
        };

        for row in &build_inline(&f, &opts).rows {
            match row.kind {
                RowKind::Header | RowKind::Gap | RowKind::Filler => {
                    assert_eq!(row.source, None, "{:?} should have no source", row.kind)
                }
                _ => assert!(row.source.is_some(), "{:?} should have a source", row.kind),
            }
        }

        // The same holds for the fillers a one-sided change puts in the other pane.
        let split = build_side_by_side(&f, &opts);
        for row in &split.right.rows {
            if row.kind == RowKind::Filler {
                assert_eq!(row.source, None);
            }
        }
    }

    #[test]
    fn side_by_side_rows_point_at_the_same_lines_as_inline_ones() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let opts = RowOptions::default();

        let inline: Vec<_> = build_inline(&f, &opts)
            .rows
            .iter()
            .filter_map(|r| r.source)
            .collect();
        let split = build_side_by_side(&f, &opts);
        let mut both: Vec<_> = split
            .left
            .rows
            .iter()
            .chain(split.right.rows.iter())
            .filter_map(|r| r.source)
            .collect();
        both.sort_by_key(|r| (r.hunk, r.line));
        both.dedup();

        let mut expected = inline;
        expected.sort_by_key(|r| (r.hunk, r.line));
        expected.dedup();
        assert_eq!(both, expected);
    }

    #[test]
    fn tabs_are_expanded_in_row_text() {
        let f = file(vec![change_hunk(1, &["\tindented"], &["\t\tdeeper"])]);
        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(rows.rows[1].text, "    indented");
        assert_eq!(rows.rows[2].text, "        deeper");
    }

    #[test]
    fn tab_width_flows_through_to_row_text() {
        let f = file(vec![change_hunk(1, &["\tx"], &["y"])]);
        let rows = build_inline(
            &f,
            &RowOptions {
                render: RenderOptions {
                    tab_width: 8,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert_eq!(rows.rows[1].text, "        x");
    }

    #[test]
    fn the_longest_line_is_measured_after_expansion() {
        // The tab makes this line four columns wide, not two.
        let f = file(vec![change_hunk(1, &["\tx"], &["ab"])]);
        let rows = build_inline(&f, &RowOptions::default());
        assert_eq!(rows.rows[1].text, "    x");
        // "context before" is 14 columns, the longest thing present.
        assert_eq!(rows.longest_line_columns, 14);
    }

    #[test]
    fn the_longest_line_accounts_for_wide_glyphs() {
        let f = file(vec![change_hunk(1, &["\u{4f60}\u{597d}"], &["x"])]);
        let mut hunk = change_hunk(1, &["\u{4f60}\u{597d}"], &["x"]);
        hunk.lines.retain(|l| l.kind != LineKind::Context);
        let rows = build_inline(&file(vec![hunk]), &RowOptions::default());
        assert_eq!(
            rows.longest_line_columns, 4,
            "two CJK glyphs are four columns"
        );
        let _ = f;
    }

    #[test]
    fn an_empty_row_set_measures_zero() {
        let rows = build_inline(&file(vec![]), &RowOptions::default());
        assert_eq!(rows.longest_line_columns, 0);
    }

    #[test]
    fn a_fetch_in_progress_splits_the_gap_and_says_so() {
        let mut f = file_with_leading_gap();
        // Lines 1 to 3 of the nine hidden ones.
        f.fetch_started(1, 3);

        let rows = build_inline(&f, &RowOptions::default());
        let gaps: Vec<&Row> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .collect();

        assert_eq!(gaps.len(), 2, "the run being fetched, then the rest");
        assert_eq!(gaps[0].gap_state, GapState::Waiting);
        assert_eq!(gaps[0].hidden_count, 3);
        assert_eq!(gaps[1].gap_state, GapState::Hidden);
        assert_eq!(gaps[1].hidden_count, 6);
    }

    #[test]
    fn a_failed_fetch_carries_its_reason() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 9, "no such revision");

        let rows = build_inline(&f, &RowOptions::default());
        let gap = rows.rows.iter().find(|r| r.kind == RowKind::Gap).unwrap();

        assert_eq!(gap.gap_state, GapState::Failed);
        assert_eq!(gap.text, "no such revision");
        assert_eq!(gap.hidden_count, 9, "the whole run is still hidden");
    }

    #[test]
    fn retrying_replaces_the_failure_rather_than_adding_to_it() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 9, "no such revision");
        f.fetch_started(1, 9);

        assert_eq!(f.fetches.len(), 1);
        let rows = build_inline(&f, &RowOptions::default());
        let gap = rows.rows.iter().find(|r| r.kind == RowKind::Gap).unwrap();
        assert_eq!(gap.gap_state, GapState::Waiting);
    }

    #[test]
    fn lines_arriving_end_the_fetch_that_asked_for_them() {
        let mut f = file_with_leading_gap();
        f.fetch_started(1, 3);
        f.expand(
            1,
            3,
            ["one", "two", "three"].iter().map(|s| (*s).to_owned()),
        );

        assert!(f.fetches.is_empty());
        let rows = build_inline(&f, &RowOptions::default());
        assert!(rows.rows.iter().all(|r| r.gap_state == GapState::Hidden));
    }

    #[test]
    fn abandoning_a_fetch_leaves_the_gap_as_it_was() {
        let mut f = file_with_leading_gap();
        f.fetch_started(1, 3);
        f.fetch_abandoned(1, 3);

        let rows = build_inline(&f, &RowOptions::default());
        let gaps: Vec<&Row> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .collect();
        assert_eq!(gaps.len(), 1, "back to one undivided run");
        assert_eq!(gaps[0].hidden_count, 9);
    }

    #[test]
    fn the_heading_stays_with_the_stretch_nearest_the_hunk_when_a_fetch_splits_it() {
        let mut f = file_with_leading_gap();
        if let FileContent::Text { hunks } = &mut f.content {
            hunks[0].heading = Some("fn thing()".to_owned());
        }
        f.fetch_started(1, 3);

        let rows = build_inline(&f, &RowOptions::default());
        let gaps: Vec<&Row> = rows
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Gap)
            .collect();
        assert_eq!(
            gaps[0].text, "",
            "the fetching stretch is not next to the hunk"
        );
        assert_eq!(gaps[1].text, "fn thing()");
    }

    #[test]
    fn side_by_side_shows_a_fetch_on_both_panes() {
        let mut f = file_with_leading_gap();
        f.fetch_started(1, 3);

        let split = build_side_by_side(&f, &RowOptions::default());
        let waiting = |rows: &Rows| {
            rows.rows
                .iter()
                .filter(|r| r.gap_state == GapState::Waiting)
                .count()
        };
        assert_eq!(waiting(&split.left), 1);
        assert_eq!(waiting(&split.right), 1);
    }
}
