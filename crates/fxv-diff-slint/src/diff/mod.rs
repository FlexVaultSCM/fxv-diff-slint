//! Everything that knows a diff is a diff.
//!
//! Three levels, in the order one feeds the next:
//!
//! - [`model`] is the document: what the unified diff said, owned and parsed.
//! - [`parse`] produces that from unified diff text.
//! - [`layout`] arranges it, deciding which position holds what once a view has chosen between
//!   an inline and a side-by-side reading.
//! - [`render`] turns that arrangement into rows, which is the last point anything knows a diff
//!   is a diff.
//!
//! What draws those rows is not here. A pane renders rows and knows nothing about additions,
//! removals, or hunks, which is what lets the same pane show a file with no diff behind it.

pub mod layout;
pub mod model;
pub mod parse;
pub mod render;

// == Internal Crates
use crate::span::Document;

impl Document {
    /// The file as it was, which a diff shows on the left.
    pub const BEFORE: Document = Document(0);
    /// The file as it is, which a diff shows on the right.
    pub const AFTER: Document = Document(1);
}
