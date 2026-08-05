//! Flattening a parsed diff into the rows a view renders.
//!
//! A unified diff describes changes; a view draws lines. This is where one becomes the other.
//! It is plain data in and plain data out, with no Slint involved, because the interesting
//! parts (aligning two sides, working out what is hidden between hunks) are worth testing
//! without a window in the way.

// == Internal Crates
use crate::model::{DiffLine, FileDiff, Hunk, LineKind};
use crate::text::{expand_tabs, DEFAULT_TAB_WIDTH};

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
    pub text: String,
    /// How many lines a gap row is hiding. Zero for every other kind.
    pub hidden_count: u32,
}

impl Row {
    fn filler() -> Self {
        Row {
            kind: RowKind::Filler,
            left_line: None,
            right_line: None,
            text: String::new(),
            hidden_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RowOptions {
    pub tab_width: usize,
    /// Prepend a row naming the file.
    pub include_file_header: bool,
    /// Total lines in the left file. Without it there is no way to know whether anything
    /// follows the last hunk, so no trailing gap is produced.
    pub left_total_lines: Option<u32>,
    /// Total lines in the right file. See `left_total_lines`.
    pub right_total_lines: Option<u32>,
}

impl Default for RowOptions {
    fn default() -> Self {
        RowOptions {
            tab_width: DEFAULT_TAB_WIDTH,
            include_file_header: false,
            left_total_lines: None,
            right_total_lines: None,
        }
    }
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
        let longest_line_columns = rows
            .iter()
            .map(|r| crate::text::display_width(&r.text) as u32)
            .max()
            .unwrap_or(0);
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

/// Builds the rows for a single-column view showing removals above additions.
pub fn build_inline(file: &FileDiff, opts: &RowOptions) -> Rows {
    let mut rows = Vec::new();

    if opts.include_file_header {
        rows.push(header_row(file));
    }

    let mut walker = GapWalker::new(opts);
    for hunk in file.hunks() {
        if let Some(gap) = walker.gap_before(hunk) {
            rows.push(gap);
        }
        for block in change_blocks(&hunk.lines) {
            match block {
                Block::Context(line) => rows.push(content_row(line, opts)),
                Block::Change { removed, added } => {
                    rows.extend(removed.iter().map(|line| content_row(line, opts)));
                    rows.extend(added.iter().map(|line| content_row(line, opts)));
                }
            }
        }
    }
    if let Some(gap) = walker.trailing_gap() {
        rows.push(gap);
    }

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

    let mut walker = GapWalker::new(opts);
    for hunk in file.hunks() {
        if let Some(gap) = walker.gap_before(hunk) {
            left.push(gap.clone());
            right.push(gap);
        }
        pair_hunk(hunk, opts, &mut left, &mut right);
    }
    if let Some(gap) = walker.trailing_gap() {
        left.push(gap.clone());
        right.push(gap);
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
fn pair_hunk(hunk: &Hunk, opts: &RowOptions, left: &mut Vec<Row>, right: &mut Vec<Row>) {
    for block in change_blocks(&hunk.lines) {
        match block {
            Block::Context(line) => {
                let row = content_row(line, opts);
                left.push(row.clone());
                right.push(row);
            }
            Block::Change { removed, added } => {
                for slot in 0..removed.len().max(added.len()) {
                    left.push(row_or_filler(removed.get(slot), opts));
                    right.push(row_or_filler(added.get(slot), opts));
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
enum Block<'a> {
    Context(&'a DiffLine),
    /// Lines taken out, and the lines put in their place. Either may be empty: a pure
    /// insertion has no removals and a pure deletion has no additions.
    Change {
        removed: &'a [DiffLine],
        added: &'a [DiffLine],
    },
}

fn change_blocks(lines: &[DiffLine]) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].kind == LineKind::Context {
            blocks.push(Block::Context(&lines[i]));
            i += 1;
            continue;
        }

        // Removals then additions, the order a unified diff writes them. Taking each run
        // independently means the reverse order costs nothing.
        let removed_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let removed = &lines[removed_start..i];

        let added_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }

        blocks.push(Block::Change {
            removed,
            added: &lines[added_start..i],
        });
    }

    blocks
}

/// Turns a parsed line into the row that renders it.
///
/// The line numbers come across unchanged rather than being cleared per side: a removed line
/// already has no right-hand number and an added line has no left-hand one.
fn content_row(line: &DiffLine, opts: &RowOptions) -> Row {
    let (text, _) = expand_tabs(&line.text, opts.tab_width);
    Row {
        kind: match line.kind {
            LineKind::Context => RowKind::Context,
            LineKind::Added => RowKind::Added,
            LineKind::Removed => RowKind::Removed,
        },
        left_line: line.left_line,
        right_line: line.right_line,
        text,
        hidden_count: 0,
    }
}

/// A row for the line, or blank space where that side of a change has run out.
fn row_or_filler(line: Option<&DiffLine>, opts: &RowOptions) -> Row {
    line.map_or_else(Row::filler, |line| content_row(line, opts))
}

fn header_row(file: &FileDiff) -> Row {
    Row {
        kind: RowKind::Header,
        left_line: None,
        right_line: None,
        text: file.display_path().to_owned(),
        hidden_count: 0,
    }
}

/// Tracks how far through each file the hunks have reached, so the lines between them can be
/// reported as gaps.
struct GapWalker {
    /// First line not yet covered, on each side. Both start at 1.
    left_next: u32,
    right_next: u32,
    left_total: Option<u32>,
    right_total: Option<u32>,
    /// Set once any hunk has been seen, so a file with no hunks produces no trailing gap.
    seen_hunk: bool,
}

impl GapWalker {
    fn new(opts: &RowOptions) -> Self {
        GapWalker {
            left_next: 1,
            right_next: 1,
            left_total: opts.left_total_lines,
            right_total: opts.right_total_lines,
            seen_hunk: false,
        }
    }

    fn gap_before(&mut self, hunk: &Hunk) -> Option<Row> {
        // Taken as the larger of the two sides. They agree wherever both files have the
        // content, and where one does not (a file that was added, so every left number is
        // zero) the other still gives the right answer.
        let left_hidden = hunk.left_start.saturating_sub(self.left_next);
        let right_hidden = hunk.right_start.saturating_sub(self.right_next);
        let hidden = left_hidden.max(right_hidden);

        let gap = (hidden > 0).then(|| Row {
            kind: RowKind::Gap,
            // Where the hidden run starts, so a host asked to expand it knows what to fetch.
            left_line: (left_hidden > 0).then_some(self.left_next),
            right_line: (right_hidden > 0).then_some(self.right_next),
            text: hunk.heading.clone().unwrap_or_default(),
            hidden_count: hidden,
        });

        self.left_next = (hunk.left_start + hunk.left_len).max(1);
        self.right_next = (hunk.right_start + hunk.right_len).max(1);
        self.seen_hunk = true;

        gap
    }

    fn trailing_gap(&self) -> Option<Row> {
        if !self.seen_hunk {
            return None;
        }

        let left_hidden = self
            .left_total
            .map_or(0, |total| (total + 1).saturating_sub(self.left_next));
        let right_hidden = self
            .right_total
            .map_or(0, |total| (total + 1).saturating_sub(self.right_next));
        let hidden = left_hidden.max(right_hidden);

        (hidden > 0).then(|| Row {
            kind: RowKind::Gap,
            left_line: (left_hidden > 0).then_some(self.left_next),
            right_line: (right_hidden > 0).then_some(self.right_next),
            text: String::new(),
            hidden_count: hidden,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffLine, FileChange, FileContent};

    fn line(kind: LineKind, left: Option<u32>, right: Option<u32>, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.to_owned(),
            left_line: left,
            right_line: right,
            no_newline_at_eof: false,
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
                tab_width: 8,
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
}
