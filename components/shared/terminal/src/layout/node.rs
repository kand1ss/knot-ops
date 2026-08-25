use indicatif::ProgressBar;
use std::ops::Deref;

#[derive(Clone, Debug)]
pub(crate) struct LayoutNode {
    id: usize,
    pub bar: ProgressBar,
}

impl LayoutNode {
    pub(crate) fn new(id: usize, bar: ProgressBar) -> Self {
        Self { id, bar }
    }
}

impl PartialEq for LayoutNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Deref for LayoutNode {
    type Target = ProgressBar;

    fn deref(&self) -> &Self::Target {
        &self.bar
    }
}
