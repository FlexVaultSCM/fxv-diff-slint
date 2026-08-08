//! What the viewer is showing, and the vocabulary for naming parts of it.
//!
//! Two identifiers and the models they name. Kept apart from both the search and the
//! application so that neither has to depend on the other to say which pane it means.

// == Internal Crates
use fxv_diff_slint::{Channel, RowModel};

/// Which tab is on screen.
///
/// The two hold different documents, so a position in one means nothing in the other: what was
/// found, which match is current, and which panes to paint are all per tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Standalone,
    Diff,
}

impl Tab {
    /// The tab a window's current index names.
    pub fn at(index: i32) -> Tab {
        if index == 0 {
            Tab::Standalone
        } else {
            Tab::Diff
        }
    }

    /// The panes this tab can draw into, whether or not each is built.
    pub fn panes(self) -> &'static [Which] {
        match self {
            Tab::Standalone => &[Which::Plain],
            Tab::Diff => &[Which::Inline, Which::Left, Which::Right],
        }
    }
}

/// Which pane, of the four the viewer can have on screen.
///
/// Ordered so that sorting by it puts the left pane of a split before the right, which is the
/// order a reader takes them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Which {
    Inline,
    Left,
    Right,
    Plain,
}

/// The row models currently on screen, kept because painting a channel goes through the model
/// that holds the rows rather than through the widget.
///
/// Only the panes the current layout uses are filled: an inline view has no left or right.
#[derive(Default)]
pub struct Panes {
    inline: Option<RowModel>,
    left: Option<RowModel>,
    right: Option<RowModel>,
    plain: Option<RowModel>,
}

impl Panes {
    pub fn get(&mut self, which: Which) -> Option<&mut RowModel> {
        match which {
            Which::Inline => self.inline.as_mut(),
            Which::Left => self.left.as_mut(),
            Which::Right => self.right.as_mut(),
            Which::Plain => self.plain.as_mut(),
        }
    }

    /// Forgets the diff panes, before rows are built to replace them.
    ///
    /// A model outliving the rows it describes would paint rows that no longer exist.
    pub fn clear_diff(&mut self) {
        self.inline = None;
        self.left = None;
        self.right = None;
    }

    pub fn set_inline(&mut self, model: RowModel) {
        self.inline = Some(model);
    }

    pub fn set_split(&mut self, left: RowModel, right: RowModel) {
        self.left = Some(left);
        self.right = Some(right);
    }

    pub fn set_plain(&mut self, model: RowModel) {
        self.plain = Some(model);
    }

    /// Every diff pane that is built, paired with which one it is.
    ///
    /// Handed back together rather than fetched one at a time because three separate
    /// `&mut self.field` borrows are not provably disjoint to the compiler, while one
    /// destructuring is.
    pub fn diff_panes(&mut self) -> impl Iterator<Item = (Which, &mut RowModel)> {
        let Panes {
            inline,
            left,
            right,
            ..
        } = self;
        [
            (Which::Inline, inline),
            (Which::Left, left),
            (Which::Right, right),
        ]
        .into_iter()
        .filter_map(|(which, model)| model.as_mut().map(|model| (which, model)))
    }

    /// Clears a channel across every built pane of one tab.
    ///
    /// Only that tab's, because both tabs are searched on every keystroke and clearing all four
    /// would have each pass undo the other's mark.
    pub fn clear_channel(&mut self, tab: Tab, channel: Channel) {
        for which in tab.panes() {
            if let Some(model) = self.get(*which) {
                model.set_channel(channel, &[]);
            }
        }
    }
}
