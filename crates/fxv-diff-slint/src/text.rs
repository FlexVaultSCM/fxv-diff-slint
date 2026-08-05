//! Turning file content into text laid out on a fixed character grid.

// == External Crates
use unicode_width::UnicodeWidthChar;

/// How many columns a tab advances to.
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// Expands tabs and measures the result, in grid columns.
///
/// Tabs advance to the next multiple of `tab_width` rather than inserting a fixed number of
/// spaces, which is what every editor does and what makes aligned code stay aligned.
///
/// The width of everything else comes from the Unicode East Asian Width tables, so a CJK
/// glyph counts as the two columns a monospace font gives it. That is not exact for
/// everything: emoji, combining marks and regional indicators can render wider or narrower
/// than their table entry suggests, and there is no way to ask the font. Lines containing
/// them will be a column or two out.
pub fn expand_tabs(line: &str, tab_width: usize) -> (String, usize) {
    // A tab width of zero would make tab stops meaningless and loop forever below.
    let tab_width = tab_width.max(1);

    let mut out = String::with_capacity(line.len());
    let mut column = 0;

    for ch in line.chars() {
        if ch == '\t' {
            let stop = (column / tab_width + 1) * tab_width;
            out.extend(std::iter::repeat_n(' ', stop - column));
            column = stop;
        } else {
            out.push(ch);
            // Control characters have no defined width; treat them as occupying nothing
            // rather than guessing, since they are not drawn either.
            column += ch.width().unwrap_or(0);
        }
    }

    (out, column)
}

/// Measures text that has already had its tabs expanded.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_advances_to_the_next_stop_not_a_fixed_distance() {
        // From column 0 a tab fills the whole stop; from column 2 it fills only the rest.
        assert_eq!(expand_tabs("\tx", 4), ("    x".to_owned(), 5));
        assert_eq!(expand_tabs("ab\tx", 4), ("ab  x".to_owned(), 5));
        assert_eq!(expand_tabs("abc\tx", 4), ("abc x".to_owned(), 5));
        // A tab landing exactly on a stop still advances a full stop.
        assert_eq!(expand_tabs("abcd\tx", 4), ("abcd    x".to_owned(), 9));
    }

    #[test]
    fn consecutive_tabs_each_advance_one_stop() {
        assert_eq!(expand_tabs("\t\tx", 4), ("        x".to_owned(), 9));
    }

    #[test]
    fn tab_width_is_configurable() {
        assert_eq!(expand_tabs("\tx", 8), ("        x".to_owned(), 9));
        assert_eq!(expand_tabs("ab\tx", 2), ("ab  x".to_owned(), 5));
    }

    #[test]
    fn a_zero_tab_width_does_not_hang() {
        assert_eq!(expand_tabs("\tx", 0), (" x".to_owned(), 2));
    }

    #[test]
    fn wide_glyphs_count_as_two_columns() {
        // The tab stop has to account for the double-width characters before it.
        let (text, width) = expand_tabs("\u{4f60}\u{597d}", 4);
        assert_eq!(width, 4, "two CJK glyphs occupy four columns");
        assert_eq!(text, "\u{4f60}\u{597d}");

        let (_, width) = expand_tabs("\u{4f60}\t x", 4);
        assert_eq!(
            width, 6,
            "the tab advances from column 2 to 4, then two more"
        );
    }

    #[test]
    fn plain_text_is_unchanged_and_measured_by_character_count() {
        let (text, width) = expand_tabs("fn main() {}", 4);
        assert_eq!(text, "fn main() {}");
        assert_eq!(width, 12);
        assert_eq!(display_width("fn main() {}"), 12);
    }

    #[test]
    fn trailing_whitespace_survives_expansion() {
        let (text, width) = expand_tabs("x  ", 4);
        assert_eq!(text, "x  ");
        assert_eq!(width, 3);
    }

    #[test]
    fn an_empty_line_is_zero_columns() {
        assert_eq!(expand_tabs("", 4), (String::new(), 0));
    }
}
