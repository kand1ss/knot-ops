pub(crate) mod anchor;
mod node;
pub(crate) use node::*;

use anchor::TaskLayoutAnchor;
use indicatif::{MultiProgress, ProgressBar};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A layout manager for organizing progress bars sequentially in a terminal.
///
/// `TaskLayout` keeps track of the rendering order of progress bars and handles
/// insertion points so that subtasks appear directly below their parent task.
pub struct TaskLayout {
    multi: MultiProgress,
    anchor: Mutex<TaskLayoutAnchor>,
    node_counter: AtomicUsize,
}
impl TaskLayout {
    pub(crate) fn new(multi: MultiProgress, anchor: TaskLayoutAnchor) -> Self {
        Self {
            multi,
            anchor: Mutex::new(anchor),
            node_counter: AtomicUsize::new(0),
        }
    }

    fn next_id(&self) -> usize {
        self.node_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn insert(&self, bar: ProgressBar) -> LayoutNode {
        let mut anchor = self.anchor.lock().unwrap();
        let inserted = if let Some(anchor_bar) = anchor.get() {
            self.multi.insert_after(anchor_bar, bar)
        } else {
            self.multi.add(bar)
        };
        let node = LayoutNode::new(self.next_id(), inserted);
        anchor.set_anchor(node.clone());
        node
    }

    pub(crate) fn insert_after(&self, target: &LayoutNode, bar: ProgressBar) -> LayoutNode {
        let mut anchor = self.anchor.lock().unwrap();

        let inserted_pb = self.multi.insert_after(&target.bar, bar);
        let node = LayoutNode::new(self.next_id(), inserted_pb);

        let is_current_anchor = match anchor.get() {
            Some(current) => current == target,
            None => false,
        };

        if is_current_anchor {
            anchor.set_anchor(node.clone());
        }

        node
    }

    pub(crate) fn set_anchor(&self, node: LayoutNode) {
        let mut anchor = self.anchor.lock().unwrap();
        anchor.set_anchor(node);
    }

    pub(crate) fn update_anchor_if_in(&self, nodes: &[LayoutNode], fallback: LayoutNode) {
        let mut anchor = self.anchor.lock().unwrap();
        if let Some(current) = anchor.get()
            && nodes.iter().any(|node| node == current)
        {
            anchor.set_anchor(fallback);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::{ProgressBar, ProgressDrawTarget};

    fn create_layout() -> TaskLayout {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
        let anchor = TaskLayoutAnchor::new(None);
        TaskLayout::new(multi, anchor)
    }

    #[test]
    fn test_layout_insert() {
        let layout = create_layout();
        let pb = ProgressBar::hidden();
        let node = layout.insert(pb);

        assert_eq!(layout.node_counter.load(Ordering::SeqCst), 1);

        let pb2 = ProgressBar::hidden();
        let node2 = layout.insert(pb2);

        assert_ne!(node, node2);
        assert_eq!(layout.node_counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_layout_insert_after() {
        let layout = create_layout();
        let node1 = layout.insert(ProgressBar::hidden());
        let node2 = layout.insert_after(&node1, ProgressBar::hidden());

        assert_ne!(node1, node2);
        assert_eq!(layout.node_counter.load(Ordering::SeqCst), 2);

        let current_anchor = layout.anchor.lock().unwrap().get().unwrap().clone();
        assert_eq!(current_anchor, node2);
    }

    #[test]
    fn test_layout_set_anchor() {
        let layout = create_layout();
        let node1 = layout.insert(ProgressBar::hidden());
        let _node2 = layout.insert(ProgressBar::hidden());

        layout.set_anchor(node1.clone());

        let current_anchor = layout.anchor.lock().unwrap().get().unwrap().clone();
        assert_eq!(current_anchor, node1);
    }
}
