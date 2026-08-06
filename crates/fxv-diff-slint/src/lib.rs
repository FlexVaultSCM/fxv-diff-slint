//! A Slint widget for viewing unified diffs, side by side or inline.
//!
//! # Using it
//!
//! Add the dependency and import the components in your `.slint`:
//!
//! ```slint,ignore
//! import { DiffView, DiffRow, DiffStyle } from "@FxvDiff";
//!
//! export component MainWindow inherits Window {
//!     in property <[DiffRow]> rows;
//!     DiffView { rows: root.rows; }
//! }
//! ```
//!
//! Nothing else is needed. In particular there is no build script wiring on your side: the
//! `@FxvDiff` import resolves through metadata this crate's build script emits, and Cargo
//! enables the Slint compiler feature that reads it.
//!
//! The generated types live in [`ui`], so your generated code refers to
//! `fxv_diff_slint::ui::DiffRow` and so on.
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
//! would make that grid vary by platform. Setting [`DiffStyle`]'s `font-family` overrides it.

pub mod model;
pub mod parse;
pub mod rows;
pub mod text;
pub mod view;

pub use model::{DiffLine, DiffSet, FileChange, FileContent, FileDiff, FileMode, Hunk, LineKind};
pub use parse::{parse_unified_diff, ParseError};
pub use rows::{build_inline, build_side_by_side, Row, RowKind, RowOptions, Rows, SideBySideRows};
pub use text::{display_column_of, map_span, render_line, source_index_at, RenderOptions};
pub use view::ViewRows;

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
    DiffLayoutMode, DiffLineRef, DiffRow, DiffRowKind, DiffSide, DiffStyle, DiffView,
    GapExpandRequest,
};
