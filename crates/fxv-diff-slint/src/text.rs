//! Turning a line of a file into the text a view draws, and back again.
//!
//! These are two different coordinate systems and conflating them is the bug this module
//! exists to prevent. A tab occupies one character in the file and up to eight columns on
//! screen; with whitespace made visible it also draws as something other than itself. So:
//!
//!   - **display text** is for drawing, and nothing else,
//!   - **copying** always resolves back to the source characters,
//!   - **spans** computed over source text are mapped before they are drawn.
//!
//! Everything here walks the line the same way, through [`layout`], so the drawing and the
//! mapping cannot disagree about where a character sits.

// == Std
use std::{iter, ops::Range, str::Chars};

// == External Crates
use unicode_width::UnicodeWidthChar;

// == Internal Crates
use crate::model::LineEnding;

/// How many columns a tab advances to.
pub const DEFAULT_TAB_WIDTH: usize = 4;

// Glyphs for whitespace made visible.
//
// All four are checked to exist in the bundled font at exactly one character advance. The
// Control Pictures block (U+240A, U+240D) would be the obvious choice for the line endings and
// is deliberately not used: it is absent from the font, and a missing glyph falls back to
// another face with a different advance, which breaks the column grid. Re-check these if the
// font is ever changed.
const TAB_MARK: char = '\u{2192}'; // rightwards arrow
const SPACE_MARK: char = '\u{00b7}'; // middle dot
const CR_MARK: char = '\u{21b5}'; // return symbol
const LF_MARK: char = '\u{00b6}'; // pilcrow

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    pub tab_width: usize,
    /// Draw spaces as dots and tabs as arrows.
    pub show_space_tabs: bool,
    /// Draw a marker for the line terminator. Separate from `show_space_tabs` because wanting
    /// one without the other is the common case.
    pub show_line_endings: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            tab_width: DEFAULT_TAB_WIDTH,
            show_space_tabs: false,
            show_line_endings: false,
        }
    }
}

/// Where one source character lands on the display grid.
///
/// Private because it is an implementation detail of the walk below: callers ask for a column
/// or an index, not for the placement each answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CharCell {
    /// Index of the character in the source line, counted in characters.
    source_index: usize,
    /// First display column the character occupies.
    column: usize,
    /// How many columns it occupies. A tab varies; a wide glyph is two; most are one.
    width: usize,
}

/// Walks a line, reporting where each character lands and which character it was.
///
/// Lazy on purpose. Everything here is built on this one walk so that drawing and mapping
/// cannot disagree about where a character sits, but the callers want a single answer each,
/// and rendering runs once per row of the file. Collecting the placements would allocate a
/// line's worth of them every time to read one number back.
struct CharCells<'a> {
    chars: Chars<'a>,
    show_space_tabs: bool,
    tab_width: usize,
    index: usize,
    column: usize,
}

impl Iterator for CharCells<'_> {
    type Item = (CharCell, char);

    fn next(&mut self) -> Option<Self::Item> {
        let ch = self.chars.next()?;

        let width = match ch {
            // Tabs advance to the next stop rather than inserting a fixed count, which is
            // what keeps aligned code aligned.
            '\t' => (self.column / self.tab_width + 1) * self.tab_width - self.column,
            ' ' if self.show_space_tabs => 1,
            // Control characters have no defined width. Treat them as occupying nothing,
            // since they are not drawn either.
            _ => ch.width().unwrap_or(0),
        };

        let cell = CharCell {
            source_index: self.index,
            column: self.column,
            width,
        };
        self.index += 1;
        self.column += width;
        Some((cell, ch))
    }
}

fn char_cells<'a>(source: &'a str, opts: &RenderOptions) -> CharCells<'a> {
    CharCells {
        chars: source.chars(),
        show_space_tabs: opts.show_space_tabs,
        // A tab width of zero would make tab stops meaningless and loop forever.
        tab_width: opts.tab_width.max(1),
        index: 0,
        column: 0,
    }
}

/// Appends what a character draws as, given how many columns it was allotted.
fn push_rendered(out: &mut String, ch: char, width: usize, opts: &RenderOptions) {
    match ch {
        '\t' if opts.show_space_tabs => {
            out.push(TAB_MARK);
            // The arrow takes the first of the tab's columns; the rest is padding.
            out.extend(iter::repeat_n(' ', width - 1));
        }
        '\t' => out.extend(iter::repeat_n(' ', width)),
        ' ' if opts.show_space_tabs => out.push(SPACE_MARK),
        _ => out.push(ch),
    }
}

/// An upper bound on the bytes rendering will produce.
///
/// A line with no tabs and no whitespace shown renders byte for byte, so the source length is
/// exact and this costs nothing. Anything else makes the output longer, and growing a `String`
/// copies what is already in it, so one pass over the bytes to size it correctly is cheaper
/// than the reallocations it avoids.
fn rendered_capacity(source: &str, opts: &RenderOptions) -> usize {
    let mut tabs = 0;
    let mut spaces = 0;
    for byte in source.bytes() {
        match byte {
            b'\t' => tabs += 1,
            b' ' => spaces += 1,
            _ => {}
        }
    }

    // A tab becomes at most a full stop's worth of spaces, or the three byte arrow plus the
    // rest of them.
    let per_tab = opts.tab_width.max(1) + 2;
    // A space shown becomes the two byte middle dot.
    let per_space = if opts.show_space_tabs { 2 } else { 1 };
    // Return and pilcrow are two bytes each.
    let ending = if opts.show_line_endings { 4 } else { 0 };

    source.len() - tabs - spaces + tabs * per_tab + spaces * per_space + ending
}

/// Renders a line for display, and reports how many columns it occupies.
/// Splits a document into lines, each with the terminator it ended on.
///
/// `str::lines` is not enough for a viewer that can show line endings: it strips the terminator
/// and does not say which one it was, so a CRLF file and an LF file come out identical and the
/// last line of a file with no trailing newline is indistinguishable from one that has it.
///
/// A document ending in a newline yields no final empty line, because there is no line there.
pub fn split_lines(source: &str) -> impl Iterator<Item = (&str, LineEnding)> {
    source.split_inclusive('\n').map(strip_terminator)
}

/// Splits off a trailing line terminator, reporting which one it was.
///
/// A carriage return counts as part of the terminator rather than as content: it is a
/// line-ending artifact, and leaving it in renders as a stray glyph or a phantom column of
/// whitespace. Which form it was is kept rather than discarded, so a viewer can show line
/// endings without having to guess.
pub fn strip_terminator(line: &str) -> (&str, LineEnding) {
    match line.strip_suffix('\n') {
        Some(rest) => match rest.strip_suffix('\r') {
            Some(rest) => (rest, LineEnding::CrLf),
            None => (rest, LineEnding::Lf),
        },
        None => (line, LineEnding::None),
    }
}

pub fn render_line(source: &str, ending: LineEnding, opts: &RenderOptions) -> (String, usize) {
    let mut text = String::with_capacity(rendered_capacity(source, opts));
    let mut columns = 0;

    for (cell, ch) in char_cells(source, opts) {
        push_rendered(&mut text, ch, cell.width, opts);
        columns = cell.column + cell.width;
    }

    if opts.show_line_endings {
        for mark in ending_marks(ending) {
            text.push(*mark);
            columns += 1;
        }
    }

    (text, columns)
}

/// How many columns a line occupies once rendered, without rendering it.
///
/// The same walk `render_line` does, minus the string it builds. Sizing a view's horizontal
/// scrolling needs the width of every line, and building every line to find out costs an
/// allocation per line to answer a question that is a sum.
///
/// Still depends on the options: showing line endings adds columns, and the tab width decides
/// how far a tab reaches. Cheap to redo when they change, since nothing is allocated.
///
/// Nothing in the crate calls this yet. Rows are rendered up front and report their own width,
/// so the only measuring is of text that already exists. It is kept because building lines
/// only for the rows on screen needs a width for the ones that are not, and that is the shape
/// the row pipeline is heading for.
pub fn measure_line(source: &str, ending: LineEnding, opts: &RenderOptions) -> usize {
    let mut columns = 0;
    for (cell, _) in char_cells(source, opts) {
        columns = cell.column + cell.width;
    }

    if opts.show_line_endings {
        columns += ending_marks(ending).len();
    }
    columns
}

/// The display column a source character starts at.
///
/// An index past the end of the line reports the column just after it, which is where a caret
/// sits when it is at the end.
pub fn display_column_of(source: &str, char_index: usize, opts: &RenderOptions) -> usize {
    let mut past_the_end = 0;

    for (cell, _) in char_cells(source, opts) {
        if cell.source_index == char_index {
            return cell.column;
        }
        past_the_end = cell.column + cell.width;
    }

    past_the_end
}

/// The source character shown at a display column.
///
/// A column landing inside a character that occupies several, such as a tab's run or a wide
/// glyph, resolves to that character. A column past the end resolves to the end of the line,
/// so a click in empty space to the right selects up to the last character rather than
/// nothing.
pub fn source_index_at(source: &str, column: usize, opts: &RenderOptions) -> usize {
    let mut past_the_end = 0;

    for (cell, _) in char_cells(source, opts) {
        if column < cell.column + cell.width {
            return cell.source_index;
        }
        past_the_end = cell.source_index + 1;
    }

    past_the_end
}

/// Maps a range of source characters to the display columns that draw them.
///
/// This is what a highlight rectangle is positioned from: word-level diff and syntax
/// highlighting both produce spans over the file's text, which is not what is on screen.
pub fn map_span(source: &str, chars: Range<usize>, opts: &RenderOptions) -> Range<usize> {
    let mut start = None;
    let mut end = None;
    let mut past_the_end = 0;

    // Both ends in one walk, since they come from the same line.
    for (cell, _) in char_cells(source, opts) {
        if cell.source_index == chars.start {
            start = Some(cell.column);
        }
        if cell.source_index == chars.end {
            end = Some(cell.column);
        }
        past_the_end = cell.column + cell.width;
    }

    let start = start.unwrap_or(past_the_end);
    let end = end.unwrap_or(past_the_end);
    start..end.max(start)
}

fn ending_marks(ending: LineEnding) -> &'static [char] {
    match ending {
        LineEnding::Lf => &[LF_MARK],
        LineEnding::CrLf => &[CR_MARK, LF_MARK],
        LineEnding::None => &[],
    }
}

/// Measures text that has already been rendered.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> RenderOptions {
        RenderOptions::default()
    }

    #[test]
    fn splitting_reports_the_terminator_each_line_ended_on() {
        let split: Vec<(&str, LineEnding)> = split_lines("a\r\nb\nc").collect();
        assert_eq!(
            split,
            vec![
                ("a", LineEnding::CrLf),
                ("b", LineEnding::Lf),
                // No terminator, which is what `\ No newline at end of file` describes.
                ("c", LineEnding::None),
            ]
        );
    }

    #[test]
    fn a_document_ending_in_a_newline_has_no_line_after_it() {
        let split: Vec<(&str, LineEnding)> = split_lines("a\n").collect();
        assert_eq!(split, vec![("a", LineEnding::Lf)]);
        assert_eq!(split_lines("").count(), 0);
    }

    #[test]
    fn a_split_line_still_renders_its_ending() {
        let opts = RenderOptions {
            show_line_endings: true,
            ..RenderOptions::default()
        };
        // The point of reporting the terminator: rendering a line split out of a document has
        // to show the same marker as rendering one that came from a diff.
        let (line, ending) = split_lines("a\n").next().unwrap();
        assert_eq!(
            render_line(line, ending, &opts),
            render_line("a", LineEnding::Lf, &opts)
        );
        assert!(render_line(line, ending, &opts).0.chars().count() > 1);
    }

    fn visible() -> RenderOptions {
        RenderOptions {
            show_space_tabs: true,
            ..Default::default()
        }
    }

    fn render(source: &str, opts: &RenderOptions) -> (String, usize) {
        render_line(source, LineEnding::Lf, opts)
    }

    // == Tab expansion

    #[test]
    fn a_tab_advances_to_the_next_stop_not_a_fixed_distance() {
        assert_eq!(render("\tx", &plain()), ("    x".to_owned(), 5));
        assert_eq!(render("ab\tx", &plain()), ("ab  x".to_owned(), 5));
        assert_eq!(render("abc\tx", &plain()), ("abc x".to_owned(), 5));
        // A tab landing exactly on a stop still advances a full stop.
        assert_eq!(render("abcd\tx", &plain()), ("abcd    x".to_owned(), 9));
    }

    #[test]
    fn consecutive_tabs_each_advance_one_stop() {
        assert_eq!(render("\t\tx", &plain()), ("        x".to_owned(), 9));
    }

    #[test]
    fn tab_width_is_configurable() {
        let wide = RenderOptions {
            tab_width: 8,
            ..Default::default()
        };
        assert_eq!(render("\tx", &wide), ("        x".to_owned(), 9));
    }

    #[test]
    fn a_zero_tab_width_does_not_hang() {
        let zero = RenderOptions {
            tab_width: 0,
            ..Default::default()
        };
        assert_eq!(render("\tx", &zero), (" x".to_owned(), 2));
    }

    #[test]
    fn wide_glyphs_count_as_two_columns() {
        let (text, columns) = render("\u{4f60}\u{597d}", &plain());
        assert_eq!(columns, 4, "two CJK glyphs occupy four columns");
        assert_eq!(text, "\u{4f60}\u{597d}");

        // The tab has to account for them: from column 2 it advances to 4.
        let (_, columns) = render("\u{4f60}\t x", &plain());
        assert_eq!(columns, 6);
    }

    #[test]
    fn trailing_whitespace_survives() {
        assert_eq!(render("x  ", &plain()), ("x  ".to_owned(), 3));
    }

    #[test]
    fn an_empty_line_is_zero_columns() {
        assert_eq!(render("", &plain()), (String::new(), 0));
    }

    // == Whitespace made visible

    #[test]
    fn spaces_and_tabs_can_be_shown_without_changing_the_grid() {
        let (plain_text, plain_columns) = render("a b\tc", &plain());
        let (marked_text, marked_columns) = render("a b\tc", &visible());

        assert_eq!(plain_text, "a b c");
        assert_eq!(marked_text, "a\u{00b7}b\u{2192}c");
        assert_eq!(
            plain_columns, marked_columns,
            "showing whitespace must not move anything"
        );
    }

    #[test]
    fn a_shown_tab_is_an_arrow_followed_by_its_padding() {
        // From column 0 with width 4: arrow plus three spaces.
        assert_eq!(render("\tx", &visible()), ("\u{2192}   x".to_owned(), 5));
        // From column 2 the tab is only two columns wide: arrow plus one space.
        assert_eq!(render("ab\tx", &visible()), ("ab\u{2192} x".to_owned(), 5));
    }

    #[test]
    fn line_endings_are_shown_independently_of_spaces_and_tabs() {
        let endings = RenderOptions {
            show_line_endings: true,
            ..Default::default()
        };
        // Spaces stay plain even though the ending is marked.
        assert_eq!(
            render_line("a b", LineEnding::Lf, &endings),
            ("a b\u{00b6}".to_owned(), 4)
        );
        assert_eq!(
            render_line("a b", LineEnding::CrLf, &endings),
            ("a b\u{21b5}\u{00b6}".to_owned(), 5)
        );
        // A file with no final newline has nothing to mark.
        assert_eq!(
            render_line("a b", LineEnding::None, &endings),
            ("a b".to_owned(), 3)
        );
    }

    #[test]
    fn line_endings_are_not_shown_by_default() {
        assert_eq!(
            render_line("a b", LineEnding::CrLf, &plain()),
            ("a b".to_owned(), 3)
        );
    }

    // == Mapping between source and display

    #[test]
    fn a_source_character_maps_to_the_column_that_draws_it() {
        // "a" then a tab to column 4, then "b".
        let source = "a\tb";
        assert_eq!(display_column_of(source, 0, &plain()), 0);
        assert_eq!(
            display_column_of(source, 1, &plain()),
            1,
            "the tab starts at 1"
        );
        assert_eq!(
            display_column_of(source, 2, &plain()),
            4,
            "b lands on the stop"
        );
        // Past the end is the column a caret would sit at.
        assert_eq!(display_column_of(source, 3, &plain()), 5);
    }

    #[test]
    fn a_column_inside_a_tab_resolves_to_the_tab() {
        // The tab occupies columns 1 to 3.
        let source = "a\tb";
        assert_eq!(source_index_at(source, 0, &plain()), 0);
        assert_eq!(source_index_at(source, 1, &plain()), 1);
        assert_eq!(source_index_at(source, 2, &plain()), 1);
        assert_eq!(source_index_at(source, 3, &plain()), 1);
        assert_eq!(source_index_at(source, 4, &plain()), 2);
    }

    #[test]
    fn a_column_inside_a_wide_glyph_resolves_to_that_glyph() {
        let source = "\u{4f60}x";
        assert_eq!(source_index_at(source, 0, &plain()), 0);
        assert_eq!(source_index_at(source, 1, &plain()), 0, "second half");
        assert_eq!(source_index_at(source, 2, &plain()), 1);
    }

    #[test]
    fn a_column_past_the_end_resolves_to_the_end_of_the_line() {
        assert_eq!(source_index_at("abc", 99, &plain()), 3);
        assert_eq!(source_index_at("", 5, &plain()), 0);
    }

    #[test]
    fn the_mapping_round_trips_for_every_character() {
        let source = "a\tbc\t\u{4f60}d  e";
        for opts in [plain(), visible()] {
            for (index, _) in source.chars().enumerate() {
                let column = display_column_of(source, index, &opts);
                assert_eq!(
                    source_index_at(source, column, &opts),
                    index,
                    "character {index} of {source:?} did not survive the round trip"
                );
            }
        }
    }

    #[test]
    fn a_span_over_source_text_maps_to_the_columns_that_draw_it() {
        // "ab" then a tab to column 4, then "cd". Selecting the tab and "c" covers
        // columns 2 to 5.
        let source = "ab\tcd";
        assert_eq!(map_span(source, 2..4, &plain()), 2..5);
        // A span over plain characters is unchanged.
        assert_eq!(map_span(source, 0..2, &plain()), 0..2);
        // An empty span stays empty.
        assert_eq!(map_span(source, 1..1, &plain()), 1..1);
    }

    #[test]
    fn showing_whitespace_does_not_move_the_mapping() {
        let source = "a b\tc";
        for index in 0..source.chars().count() {
            assert_eq!(
                display_column_of(source, index, &plain()),
                display_column_of(source, index, &visible()),
                "character {index} moved when whitespace was shown"
            );
        }
    }

    #[test]
    fn rendered_text_measures_as_many_columns_as_the_layout_reported() {
        for source in ["", "plain", "a\tb", "\u{4f60}\u{597d}x", "  trailing  "] {
            for opts in [plain(), visible()] {
                let (text, columns) = render(source, &opts);
                assert_eq!(
                    display_width(&text),
                    columns,
                    "{source:?} disagreed about its own width"
                );
            }
        }
    }

    #[test]
    fn measuring_agrees_with_rendering() {
        // The two walk the same cells, so they must never disagree. If they do, the horizontal
        // scroll range and the text it is meant to cover part ways.
        let lines = [
            "plain text",
            "\tleading tab",
            "trailing spaces   ",
            "mixed\tand  spaced\ttext",
            "",
            "unicode: \u{4e2d}\u{6587} wide",
            // The running total is only ever as right as its last cell, so the cases that
            // matter are the ones ending in something wider than one column.
            "ends on a tab\t",
            "ends wide \u{4e2d}",
        ];
        let options = [
            RenderOptions::default(),
            RenderOptions {
                show_space_tabs: true,
                ..RenderOptions::default()
            },
            RenderOptions {
                show_line_endings: true,
                ..RenderOptions::default()
            },
            RenderOptions {
                tab_width: 8,
                show_space_tabs: true,
                show_line_endings: true,
            },
        ];

        for line in lines {
            for opts in &options {
                for ending in [LineEnding::Lf, LineEnding::CrLf, LineEnding::None] {
                    let (_, rendered) = render_line(line, ending, opts);
                    assert_eq!(
                        measure_line(line, ending, opts),
                        rendered,
                        "line {line:?} with {opts:?} and {ending:?}"
                    );
                }
            }
        }
    }
}
