//! Everything that knows a diff is a diff.
//!
//! Three levels, in the order one feeds the next:
//!
//! - [`model`] is the document: what the unified diff said, owned and parsed.
//! - [`parse`] produces that from unified diff text.
//! - [`layout`] arranges it, deciding which position holds what once a view has chosen between
//!   an inline and a side-by-side reading.
//!
//! What draws the result is not here. A pane renders rows and knows nothing about additions,
//! removals, or hunks, which is what lets the same pane show a file with no diff behind it.

pub mod layout;
pub mod model;
pub mod parse;
