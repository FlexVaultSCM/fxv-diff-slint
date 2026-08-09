//! Rows as the widget draws them.
//!
//! A layout says which line belongs at which position. A pane has to answer more than that:
//! what the line reads as once tabs are expanded, how many columns it occupies, which numbers
//! its gutter shows, and what is painted behind it. Those are all decisions about display.
//!
//! Nothing here knows what a diff is. Rows arrive already decided, whether they came from a
//! laid-out diff or from a plain file, and everything past that point works the same either
//! way. What turns a diff into rows lives with the diff.
//!
//! Still no pixels. A column becomes a position on screen by multiplying by the character
//! advance, and that happens in the markup.

pub mod model;
pub mod row;

pub use model::RowModel;
pub use row::{Channel, DisplayColumnExtent, DisplayedRow, Highlight, RowClass, GUTTER_COLUMNS};
