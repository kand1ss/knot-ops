use crate::layout::LayoutNode;

/// Holds a reference to the active layout node serving as the insertion anchor.
///
/// New child progress bars are inserted relative to this anchor.
pub struct TaskLayoutAnchor {
    anchor: Option<LayoutNode>,
}

impl TaskLayoutAnchor {
    pub(crate) fn new(anchor: Option<LayoutNode>) -> Self {
        Self { anchor }
    }

    pub(crate) fn set_anchor(&mut self, node: LayoutNode) {
        self.anchor = Some(node);
    }

    pub(crate) fn get(&self) -> Option<&LayoutNode> {
        self.anchor.as_ref()
    }
}
