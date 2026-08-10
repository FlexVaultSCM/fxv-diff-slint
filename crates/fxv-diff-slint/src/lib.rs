//! A Slint widget for viewing unified diffs, side by side or inline.
//!
//! # Using it
//!
//! Add the dependency and import the components in your `.slint`:
//!
//! ```slint,ignore
//! import { DiffView, LinePair, CodeStyle } from "@FxvDiff";
//!
//! export component MainWindow inherits Window {
//!     in property <[LinePair]> rows;
//!     DiffView { rows: root.rows; }
//! }
//! ```
//!
//! Nothing else is needed. In particular there is no build script wiring on your side: the
//! `@FxvDiff` import resolves through metadata this crate's build script emits, and Cargo
//! enables the Slint compiler feature that reads it.
//!
//! The generated types live in [`ui`], so your generated code refers to
//! `fxv_diff_slint::ui::CodeRow` and so on.
//!
//! # Renderers
//!
//! This crate enables no Slint backend or renderer. That is the application's choice, not a
//! widget library's, so your binary picks one as usual.
//!
//! # Fonts
//!
//! A monospace font is embedded and used by default. The widget positions line numbers,
//! highlights and text on a character grid measured from the font, so a system font lookup
//! would make that grid vary by platform. Setting [`CodeStyle`]'s `font-family` overrides it.

pub mod diff;
pub mod highlight;
pub mod selection;
pub mod span;
#[cfg(test)]
mod test_fixtures;
pub mod text;
pub mod view;

// Re-exported flat, so a consumer names a type once rather than tracking which level of the
// diff it belongs to.
pub use diff::layout::{
    build_inline, build_split, GapState, Layout, Line, LinePair, Row, RowOptions,
};
pub use diff::model::{
    DiffLine, DiffSet, Fetch, FetchState, FileChange, FileContent, FileDiff, FileMode, Hunk,
    LineKind, LineOrigin,
};
pub use diff::parse::{parse_unified_diff, ParseError};
pub use diff::render::{render_diff, DiffPane};
// A selection travels through three forms: the gesture, the durable spans it resolves to, and
// the ranges those are painted as. One module owns each.
pub use highlight::to_highlights;
pub use selection::{clamp_to_run, run_bounds, to_spans, Caret, Selection};
pub use span::{Document, LineSpan, SourceCharExtent};
pub use text::{
    display_column_of, map_span, measure_line, render_line, source_index_at, split_lines,
    strip_terminator, LineEnding, RenderOptions,
};
pub use view::{Channel, DisplayColumnExtent, Highlight};
pub use view::{DisplayedRow, Gap, RowClass, RowModel, GUTTER_COLUMNS};

// Machine-generated, and it names everything through full paths. That is correct for
// generated code and not something to lint.
#[allow(clippy::absolute_paths)]
pub mod ui {
    //! Types generated from the crate's `.slint` sources.
    //!
    //! Rarely named directly: the code Slint generates for your own `.slint` files refers to
    //! it for you.
    slint::include_modules!();
}

pub use ui::{
    CodeGapExpandRequest, CodeGapState, CodeLine, CodePosition, CodeRow, CodeSelection, CodeStyle,
    DiffView,
};
