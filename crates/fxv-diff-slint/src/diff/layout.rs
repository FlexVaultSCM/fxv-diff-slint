//! Laying a parsed diff out as a sequence of rows.
//!
//! A unified diff describes changes; a view shows a list. This is where one becomes the other,
//! and it stops short of anything about drawing. A row says which line of which file belongs at
//! a position, not what that line looks like: no rendered text, no column counts, no options
//! about tabs or whitespace. Those belong to whatever puts it on screen, and keeping them out
//! means changing how a line is drawn does not disturb where it sits.
//!
//! What the types are, and how they nest:
//!
//! ```text
//! Layout                    one diff, arranged for one view
//!  |
//!  +- Row                   one position in that arrangement, drawn as one row per pane
//!      |
//!      +- Lines(LinePair)   content
//!      |    |
//!      |    +- left:  Option<Line>    from the before-file, absent if the line was added
//!      |    +- right: Option<Line>    from the after-file, absent if it was removed
//!      |
//!      +- Gap { .. }        lines that exist but are not shown, and what is happening to them
//!      +- Header            the file itself rather than anything in it
//!
//! Line { number, source }   a line number, and the way back to its text
//! ```
//!
//! `Row` here is the arrangement's row; `view::DisplayedRow` is what one turns into once a pane
//! has decided how it looks. `Block`, further down, is a private grouping used while building
//! and never leaves this module.
//!
//! Plain data in and plain data out, with no Slint involved, because the interesting parts,
//! aligning two sides and working out what is hidden between hunks, are worth testing without a
//! window in the way.

// == Std
use std::ops::Range;

// == Internal Crates
use super::model::{DiffLine, Fetch, FetchState, FileDiff, Hunk, LineKind, LineRef};

/// Why a gap's lines are not on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GapState {
    /// Nobody has asked for them.
    #[default]
    Hidden,
    /// Somebody has, and they have not arrived.
    Waiting,
    /// Somebody did, and it did not work.
    Failed,
}

/// One line of a file, at the position it was laid out.
///
/// Deliberately small. A line is a number and a way back to its text; everything else about it
/// belongs either to the document it came from or to the view drawing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    /// 1-based, in whichever file this row belongs to.
    pub line: u32,
    pub source: LineRef,
}

/// One position of a diff, holding whichever side has a line there.
///
/// Named for what it is rather than "row", because the widget's own row struct already carries
/// that name and this is not one: it is the pair a row is drawn from.
///
/// Both sides present means either an unchanged line, which is one line shown twice, or a
/// removal paired with the addition that replaced it. Those are told apart by whether the
/// sources match, and a view needing to know looks up the kind of each line rather than being
/// told: one kind cannot describe a position that is a removal on one side and an addition on
/// the other.
///
/// One side absent is a line with no counterpart, which a two-column view draws as blank space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinePair {
    pub left: Option<Line>,
    pub right: Option<Line>,
}

/// One position in a laid-out diff.
///
/// A sum rather than a row with mostly unused fields: a gap stands for lines that are not
/// there, so it has no line numbers to give, and a header stands for the file rather than for
/// anything in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Lines(LinePair),
    Gap {
        /// Where the hidden run starts on each side, so opening it knows what to ask for.
        left_start: u32,
        right_start: u32,
        hidden: u32,
        state: GapState,
        /// The run being fetched, numbered on the right, when one is.
        ///
        /// Which control started it is not recorded, because the model has no idea what a
        /// control is. A view works it out by comparing this against what each of its controls
        /// would have asked for, which survives the row being rebuilt underneath it.
        pending: Option<(u32, u32)>,
        /// The heading of the hunk this gap runs up to, when it is the stretch nearest one.
        heading: Option<String>,
        /// Why a fetch failed, when one did.
        reason: Option<String>,
    },
    Header,
}

/// A diff laid out for one arrangement.
///
/// Which arrangement matters, because the two order their entries differently: an inline view
/// lists a change's removals and then its additions, while a two-column view puts them opposite
/// each other. One list serves both panes of a two-column view, since a pane reads its own side
/// of each entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub rows: Vec<Row>,
}

/// What to lay out, as opposed to how to draw it.
#[derive(Debug, Clone, Default)]
pub struct RowOptions {
    /// Prepend an entry naming the file.
    pub include_file_header: bool,
    /// Total lines in the left file. Without it there is no way to know whether anything
    /// follows the last hunk, so no trailing gap is produced.
    pub left_total_lines: Option<u32>,
    /// Total lines in the right file. See `left_total_lines`.
    pub right_total_lines: Option<u32>,
}

/// Lays a diff out in one column, with a change's removals above its additions.
pub fn build_inline(file: &FileDiff, opts: &RowOptions) -> Layout {
    let mut rows = Vec::new();

    if opts.include_file_header {
        rows.push(Row::Header);
    }

    let mut walker = GapWalker::new(file, opts);
    for (h, hunk) in file.hunks().iter().enumerate() {
        rows.extend(walker.gap_before(hunk));
        for block in change_blocks(&hunk.lines) {
            match block {
                Block::Context(i) => rows.push(context_entry(h, i, &hunk.lines)),
                Block::Change { removed, added } => {
                    // One run after the other, which is what makes this the inline
                    // arrangement rather than the paired one.
                    rows.extend(removed.map(|i| one_sided_entry(h, i, &hunk.lines)));
                    rows.extend(added.map(|i| one_sided_entry(h, i, &hunk.lines)));
                }
            }
        }
    }
    rows.extend(walker.trailing_gap());

    Layout { rows }
}

/// Lays a diff out with the two files opposite each other.
///
/// The two runs of a change are placed side by side and the shorter is padded, which is what
/// keeps a three-line removal facing the five-line addition that replaced it.
pub fn build_split(file: &FileDiff, opts: &RowOptions) -> Layout {
    let mut rows = Vec::new();

    if opts.include_file_header {
        rows.push(Row::Header);
    }

    let mut walker = GapWalker::new(file, opts);
    for (h, hunk) in file.hunks().iter().enumerate() {
        rows.extend(walker.gap_before(hunk));
        for block in change_blocks(&hunk.lines) {
            match block {
                Block::Context(i) => rows.push(context_entry(h, i, &hunk.lines)),
                Block::Change { removed, added } => {
                    for slot in 0..removed.len().max(added.len()) {
                        let l = removed.start.checked_add(slot).filter(|i| *i < removed.end);
                        let r = added.start.checked_add(slot).filter(|i| *i < added.end);
                        rows.push(Row::Lines(LinePair {
                            left: l.and_then(|i| row_at(h, i, &hunk.lines)),
                            right: r.and_then(|i| row_at(h, i, &hunk.lines)),
                        }));
                    }
                }
            }
        }
    }
    rows.extend(walker.trailing_gap());

    Layout { rows }
}

/// An unchanged line, which stands on both sides at once.
fn context_entry(hunk: usize, index: usize, lines: &[DiffLine]) -> Row {
    let source = LineRef {
        hunk: hunk as u32,
        line: index as u32,
    };
    Row::Lines(LinePair {
        left: lines[index].left_line.map(|line| Line { line, source }),
        right: lines[index].right_line.map(|line| Line { line, source }),
    })
}

/// A removal or an addition, which exists on one side only.
fn one_sided_entry(hunk: usize, index: usize, lines: &[DiffLine]) -> Row {
    let row = row_at(hunk, index, lines);
    Row::Lines(match lines[index].kind {
        LineKind::Removed => LinePair {
            left: row,
            right: None,
        },
        _ => LinePair {
            left: None,
            right: row,
        },
    })
}

/// The row for one line of a hunk, numbered by whichever side carries it.
fn row_at(hunk: usize, index: usize, lines: &[DiffLine]) -> Option<Line> {
    let line = &lines[index];
    let number = match line.kind {
        LineKind::Removed => line.left_line,
        _ => line.right_line,
    }?;
    Some(Line {
        line: number,
        source: LineRef {
            hunk: hunk as u32,
            line: index as u32,
        },
    })
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

/// Groups a hunk's lines into unchanged ones and change blocks.
///
/// "Change block" is the usual name for a run of removals followed by the additions that
/// replaced them, and it is the unit both arrangements care about: inline lists the two runs
/// one after the other, a split puts them opposite each other.
fn change_blocks(lines: &[DiffLine]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].kind == LineKind::Context {
            blocks.push(Block::Context(i));
            i += 1;
            continue;
        }

        // Not context, so a change block starts here and runs until the next context line.

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

/// What is happening to one run of hidden lines.
///
/// A named struct rather than a tuple, because three optional-looking fields in a row are
/// impossible to read at a call site: it is otherwise `(GapState, Option<(u32, u32)>,
/// Option<String>)` and nothing says which is which.
#[derive(Debug, Default)]
struct GapProgress {
    state: GapState,
    /// The run being fetched, numbered on the right, when one is.
    pending: Option<(u32, u32)>,
    /// Why the last attempt failed, when one did.
    reason: Option<String>,
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

        let rows = self.gaps(hidden, hunk.heading.as_deref());

        // Past this hunk on both sides, ready for the distance to the next one.

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

        self.gaps(hidden, None)
    }

    /// One entry per run of hidden lines, whatever is being fetched from it.
    ///
    /// A fetch belongs to the control that asked for it, not to a slice of the gap. Splitting
    /// the run around it made the gap grow a row on every attempt, and a failed attempt left
    /// that row behind for good. So the run stays whole and carries what is happening to it.
    fn gaps(&self, hidden: u32, heading: Option<&str>) -> Vec<Row> {
        if hidden == 0 {
            return Vec::new();
        }

        let progress = self.fetch_state(self.right_next, hidden);
        vec![Row::Gap {
            left_start: self.left_next,
            right_start: self.right_next,
            hidden,
            state: progress.state,
            pending: progress.pending,
            heading: heading.map(str::to_owned),
            reason: progress.reason,
        }]
    }

    /// What is happening to a run of hidden lines.
    ///
    /// A fetch in flight wins over a past failure, so starting another attempt replaces the
    /// message rather than showing both at once.
    fn fetch_state(&self, right_start: u32, count: u32) -> GapProgress {
        let overlapping = self.fetches.iter().filter(|f| {
            f.right_start < right_start + count && right_start < f.right_start + f.count
        });

        // A run still in flight wins outright: it is the newer fact, and showing both at once
        // would put a stale message beside a live spinner.
        let mut failure = None;
        for fetch in overlapping {
            match &fetch.state {
                FetchState::Waiting => {
                    return GapProgress {
                        state: GapState::Waiting,
                        pending: Some((fetch.right_start, fetch.count)),
                        reason: None,
                    }
                }
                FetchState::Failed(why) => failure = Some(why.clone()),
            }
        }

        match failure {
            Some(why) => GapProgress {
                state: GapState::Failed,
                pending: None,
                reason: Some(why),
            },
            None => GapProgress::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::model::{FileChange, FileContent, LineEnding, LineOrigin};

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

    /// A hunk with one context line, then the removals, then the additions, then one more
    /// context line. It therefore covers `removed.len() + 2` lines on the left.
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
        let after = start + removed.len() as u32 + 1;
        lines.push(line(
            LineKind::Context,
            Some(after),
            Some(start + added.len() as u32 + 1),
            "context after",
        ));

        Hunk {
            left_start: start,
            left_len: removed.len() as u32 + 2,
            right_start: start,
            right_len: added.len() as u32 + 2,
            heading: None,
            lines,
        }
    }

    /// Which sides each entry carries, as a shorthand for asserting arrangement.
    fn sides(layout: &Layout) -> Vec<(bool, bool)> {
        layout
            .rows
            .iter()
            .filter_map(|e| match e {
                Row::Lines(pair) => Some((pair.left.is_some(), pair.right.is_some())),
                _ => None,
            })
            .collect()
    }

    fn gap_counts(layout: &Layout) -> Vec<u32> {
        layout
            .rows
            .iter()
            .filter_map(|e| match e {
                Row::Gap { hidden, .. } => Some(*hidden),
                _ => None,
            })
            .collect()
    }

    fn file_with_leading_gap() -> FileDiff {
        file(vec![change_hunk(10, &["old"], &["new"])])
    }

    // == Arrangement

    #[test]
    fn inline_lists_removals_before_the_additions_that_replaced_them() {
        let f = file(vec![change_hunk(1, &["a", "b"], &["c"])]);
        assert_eq!(
            sides(&build_inline(&f, &RowOptions::default())),
            vec![
                (true, true),  // context before
                (true, false), // removed a
                (true, false), // removed b
                (false, true), // added c
                (true, true),  // context after
            ]
        );
    }

    #[test]
    fn a_split_puts_a_replacement_opposite_what_it_replaced() {
        let f = file(vec![change_hunk(1, &["a"], &["c"])]);
        assert_eq!(
            sides(&build_split(&f, &RowOptions::default())),
            vec![(true, true), (true, true), (true, true)],
            "the change occupies one position with a line on each side"
        );
    }

    #[test]
    fn a_split_pads_the_shorter_side_of_an_uneven_change() {
        let f = file(vec![change_hunk(1, &["a", "b", "c"], &["x"])]);
        assert_eq!(
            sides(&build_split(&f, &RowOptions::default())),
            vec![
                (true, true),  // context
                (true, true),  // a against x
                (true, false), // b against nothing
                (true, false), // c against nothing
                (true, true),  // context
            ]
        );
    }

    #[test]
    fn a_pure_deletion_faces_nothing() {
        let f = file(vec![change_hunk(1, &["gone"], &[])]);
        assert_eq!(
            sides(&build_split(&f, &RowOptions::default())),
            vec![(true, true), (true, false), (true, true)]
        );
    }

    #[test]
    fn an_unchanged_line_stands_on_both_sides_in_either_arrangement() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        for layout in [
            build_inline(&f, &RowOptions::default()),
            build_split(&f, &RowOptions::default()),
        ] {
            let Row::Lines(first) = &layout.rows[0] else {
                panic!("expected a line");
            };
            assert!(first.left.is_some() && first.right.is_some());
            assert_eq!(
                first.left.unwrap().source,
                first.right.unwrap().source,
                "the same line, shown twice"
            );
        }
    }

    #[test]
    fn the_line_numbers_come_through() {
        let f = file(vec![change_hunk(10, &["old"], &["new"])]);
        let layout = build_inline(&f, &RowOptions::default());
        let Row::Lines(removed) = &layout.rows[2] else {
            panic!("expected the removal");
        };
        assert_eq!(removed.left.unwrap().line, 11);
        assert!(removed.right.is_none());
    }

    #[test]
    fn a_file_header_is_opt_in() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        assert!(!build_inline(&f, &RowOptions::default())
            .rows
            .iter()
            .any(|e| matches!(e, Row::Header)));
        assert!(matches!(
            build_inline(
                &f,
                &RowOptions {
                    include_file_header: true,
                    ..RowOptions::default()
                }
            )
            .rows[0],
            Row::Header
        ));
    }

    #[test]
    fn a_file_with_no_hunks_lays_out_as_nothing() {
        let layout = build_inline(&file(vec![]), &RowOptions::default());
        assert!(layout.rows.is_empty());
    }

    // == Gaps

    #[test]
    fn a_hunk_that_does_not_start_at_line_one_is_preceded_by_a_gap() {
        let f = file_with_leading_gap();
        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![9]
        );
    }

    #[test]
    fn a_hunk_starting_at_line_one_has_no_gap_before_it() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        assert!(gap_counts(&build_inline(&f, &RowOptions::default())).is_empty());
    }

    #[test]
    fn the_lines_between_two_hunks_become_a_gap() {
        let f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(20, &["c"], &["d"]),
        ]);
        // The first hunk covers lines 1 to 3, so 4 up to 19 are hidden.
        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![16]
        );
    }

    #[test]
    fn adjacent_hunks_produce_no_gap() {
        // The first covers lines 1 to 3, so a hunk starting at 4 hides nothing.
        let f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(4, &["c"], &["d"]),
        ]);
        assert!(gap_counts(&build_inline(&f, &RowOptions::default())).is_empty());
    }

    #[test]
    fn a_trailing_gap_needs_the_file_length_to_be_known() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        assert!(gap_counts(&build_inline(&f, &RowOptions::default())).is_empty());

        let with_totals = build_inline(
            &f,
            &RowOptions {
                left_total_lines: Some(10),
                right_total_lines: Some(10),
                ..RowOptions::default()
            },
        );
        assert_eq!(gap_counts(&with_totals), vec![7]);
    }

    #[test]
    fn a_hunk_reaching_the_end_of_the_file_has_no_trailing_gap() {
        let f = file(vec![change_hunk(1, &["a"], &["b"])]);
        let layout = build_inline(
            &f,
            &RowOptions {
                left_total_lines: Some(3),
                right_total_lines: Some(3),
                ..RowOptions::default()
            },
        );
        assert!(gap_counts(&layout).is_empty());
    }

    #[test]
    fn an_added_file_has_no_leading_gap_despite_its_zero_left_numbers() {
        // Every left number is absent, so only the right side says where the hunk starts.
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
        assert!(gap_counts(&build_inline(&file(vec![hunk]), &RowOptions::default())).is_empty());
    }

    #[test]
    fn a_gap_carries_the_heading_of_the_hunk_it_precedes() {
        let mut hunk = change_hunk(10, &["old"], &["new"]);
        hunk.heading = Some("fn thing()".to_owned());
        let layout = build_inline(&file(vec![hunk]), &RowOptions::default());

        let Row::Gap { heading, .. } = &layout.rows[0] else {
            panic!("expected a gap");
        };
        assert_eq!(heading.as_deref(), Some("fn thing()"));
    }

    #[test]
    fn a_gap_says_where_its_hidden_run_starts() {
        let layout = build_inline(&file_with_leading_gap(), &RowOptions::default());
        let Row::Gap {
            left_start,
            right_start,
            ..
        } = &layout.rows[0]
        else {
            panic!("expected a gap");
        };
        assert_eq!((*left_start, *right_start), (1, 1));
    }

    #[test]
    fn a_split_lays_out_a_gap_once_rather_than_per_side() {
        // One list serves both panes, so a gap is one entry that each pane draws.
        let f = file_with_leading_gap();
        assert_eq!(
            gap_counts(&build_split(&f, &RowOptions::default())),
            vec![9]
        );
    }

    // == Fetches

    fn gap_states(layout: &Layout) -> Vec<(u32, GapState)> {
        layout
            .rows
            .iter()
            .filter_map(|e| match e {
                Row::Gap { hidden, state, .. } => Some((*hidden, *state)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_fetch_in_progress_leaves_the_gap_whole_and_says_it_is_waiting() {
        // The run stays one entry however much of it is being fetched. Splitting it grew the
        // gap a row per attempt, and a failed attempt left that row behind for good.
        let mut f = file_with_leading_gap();
        f.fetch_started(1, 3);

        assert_eq!(
            gap_states(&build_inline(&f, &RowOptions::default())),
            vec![(9, GapState::Waiting)]
        );
    }

    #[test]
    fn repeated_failures_do_not_grow_the_gap() {
        // Three attempts on different parts of the same run, all failing. The gap is still one
        // row, because a fetch belongs to the control that asked for it rather than to a slice
        // of the lines.
        let mut f = file_with_leading_gap();
        for start in [1, 4, 7] {
            f.fetch_failed(start, 3, "no source");
        }

        assert_eq!(
            gap_states(&build_inline(&f, &RowOptions::default())),
            vec![(9, GapState::Failed)]
        );
    }

    #[test]
    fn a_fetch_in_flight_outranks_a_past_failure() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 3, "no source");
        f.fetch_started(4, 3);

        assert_eq!(
            gap_states(&build_inline(&f, &RowOptions::default())),
            vec![(9, GapState::Waiting)],
            "one thing at a time, and the newer one wins"
        );
    }

    #[test]
    fn clearing_failures_returns_the_gap_to_plain_hidden_lines() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 9, "no source");
        f.clear_failed_fetches();

        assert_eq!(
            gap_states(&build_inline(&f, &RowOptions::default())),
            vec![(9, GapState::Hidden)]
        );
    }

    #[test]
    fn a_failed_fetch_carries_its_reason() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 9, "no such revision");

        let layout = build_inline(&f, &RowOptions::default());
        let Row::Gap { state, reason, .. } = &layout.rows[0] else {
            panic!("expected a gap");
        };
        assert_eq!(*state, GapState::Failed);
        assert_eq!(reason.as_deref(), Some("no such revision"));
    }

    #[test]
    fn retrying_replaces_the_failure_rather_than_adding_to_it() {
        let mut f = file_with_leading_gap();
        f.fetch_failed(1, 9, "no such revision");
        f.fetch_started(1, 9);

        assert_eq!(f.fetches.len(), 1);
        let layout = build_inline(&f, &RowOptions::default());
        let Row::Gap { state, .. } = &layout.rows[0] else {
            panic!("expected a gap");
        };
        assert_eq!(*state, GapState::Waiting);
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
        let layout = build_inline(&f, &RowOptions::default());
        assert!(layout.rows.iter().all(|e| !matches!(
            e,
            Row::Gap {
                state: GapState::Waiting,
                ..
            }
        )));
    }

    #[test]
    fn abandoning_a_fetch_leaves_the_gap_as_it_was() {
        let mut f = file_with_leading_gap();
        f.fetch_started(1, 3);
        f.fetch_abandoned(1, 3);

        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![9],
            "back to one undivided run"
        );
    }

    #[test]
    fn a_gap_keeps_its_heading_while_a_fetch_is_in_flight() {
        let mut hunk = change_hunk(10, &["old"], &["new"]);
        hunk.heading = Some("fn thing()".to_owned());
        let mut f = file(vec![hunk]);
        f.fetch_started(1, 3);

        let layout = build_inline(&f, &RowOptions::default());
        let headings: Vec<Option<String>> = layout
            .rows
            .iter()
            .filter_map(|e| match e {
                Row::Gap { heading, .. } => Some(heading.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(
            headings,
            vec![Some("fn thing()".to_owned())],
            "one gap, so the heading has nowhere else to go"
        );
    }

    // == Opening a gap

    #[test]
    fn supplied_lines_replace_the_part_of_a_gap_they_cover() {
        let mut f = file_with_leading_gap();
        f.expand(
            1,
            1,
            ["one", "two", "three"].iter().map(|s| (*s).to_owned()),
        );

        let layout = build_inline(&f, &RowOptions::default());
        assert_eq!(gap_counts(&layout), vec![6], "nine less the three opened");
    }

    #[test]
    fn a_gap_can_be_opened_from_the_far_end() {
        let mut f = file_with_leading_gap();
        f.expand(
            7,
            7,
            ["seven", "eight", "nine"].iter().map(|s| (*s).to_owned()),
        );

        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![6]
        );
    }

    #[test]
    fn opening_the_middle_of_a_gap_leaves_one_on_each_side() {
        let mut f = file_with_leading_gap();
        f.expand(4, 4, ["four", "five"].iter().map(|s| (*s).to_owned()));

        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![3, 4]
        );
    }

    #[test]
    fn a_fully_opened_gap_leaves_none() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, (1..=9).map(|n| format!("line {n}")));

        assert!(gap_counts(&build_inline(&f, &RowOptions::default())).is_empty());
    }

    #[test]
    fn asking_twice_for_the_same_lines_changes_nothing() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);
        let once = build_inline(&f, &RowOptions::default());

        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);
        assert_eq!(build_inline(&f, &RowOptions::default()), once);
    }

    #[test]
    fn opening_the_whole_gap_merges_the_hunks_it_separated() {
        let mut f = file(vec![
            change_hunk(1, &["a"], &["b"]),
            change_hunk(20, &["c"], &["d"]),
        ]);
        assert_eq!(f.hunks().len(), 2);

        f.expand(4, 4, (4..20).map(|n| format!("line {n}")));

        assert_eq!(f.hunks().len(), 1, "nothing separates them any more");
        assert!(gap_counts(&build_inline(&f, &RowOptions::default())).is_empty());
    }

    #[test]
    fn lines_far_from_any_hunk_become_a_hunk_of_their_own() {
        let mut f = file_with_leading_gap();
        // Line 40 is nowhere near the hidden range, and nothing would normally ask for it.
        // Adding it anyway is honest: the caller said to show that line, so it is shown, with
        // the distance to it left as a gap like any other.
        f.expand(40, 40, ["stray".to_owned()]);

        assert_eq!(
            gap_counts(&build_inline(&f, &RowOptions::default())),
            vec![9, 27],
            "before the first hunk, and before line 40"
        );
    }

    #[test]
    fn opening_a_gap_on_a_split_opens_it_for_both_sides_at_once() {
        let mut f = file_with_leading_gap();
        f.expand(1, 1, ["one".to_owned(), "two".to_owned()]);

        let layout = build_split(&f, &RowOptions::default());
        assert_eq!(gap_counts(&layout), vec![7]);
        // The opened lines are unchanged content, so they stand on both sides.
        let opened: Vec<(bool, bool)> = sides(&layout).into_iter().take(2).collect();
        assert_eq!(opened, vec![(true, true), (true, true)]);
    }

    // == Source references

    #[test]
    fn a_row_can_find_the_line_it_was_laid_out_from() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        let layout = build_inline(&f, &RowOptions::default());

        let Row::Lines(removed) = &layout.rows[1] else {
            panic!("expected the removal");
        };
        let source = removed.left.unwrap().source;
        assert_eq!(f.line(source).unwrap().text, "old");
    }

    #[test]
    fn both_arrangements_point_at_the_same_lines() {
        let f = file(vec![change_hunk(1, &["old"], &["new"])]);
        // Both sides, because a split puts a removal and the addition replacing it at one
        // position while inline gives them a position each.
        let sources = |layout: Layout| -> Vec<LineRef> {
            let mut all: Vec<LineRef> = layout
                .rows
                .iter()
                .filter_map(|e| match e {
                    Row::Lines(pair) => Some(pair),
                    _ => None,
                })
                .flat_map(|pair| pair.left.into_iter().chain(pair.right))
                .map(|row| row.source)
                .collect();
            all.sort_by_key(|s| (s.hunk, s.line));
            all.dedup();
            all
        };

        assert_eq!(
            sources(build_inline(&f, &RowOptions::default())),
            sources(build_split(&f, &RowOptions::default())),
            "the same lines, arranged differently"
        );
    }
}
