use crate::layout::{TaskLayout, anchor::TaskLayoutAnchor};
use indicatif::{MultiProgress, ProgressDrawTarget};
use std::sync::Arc;

/// Creates a helper layout anchor.
pub fn anchor() -> TaskLayoutAnchor {
    TaskLayoutAnchor::new(None)
}

/// Creates a hidden `MultiProgress` instance for tests.
pub fn hidden_multi() -> MultiProgress {
    MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
}

/// Creates a hidden layout wrapped in an `Arc`.
pub fn hidden_layout() -> Arc<TaskLayout> {
    Arc::new(TaskLayout::new(hidden_multi(), anchor()))
}

/// Creates a layout with a given `MultiProgress` instance wrapped in an `Arc`.
pub fn layout(multi: MultiProgress) -> Arc<TaskLayout> {
    Arc::new(TaskLayout::new(multi, anchor()))
}
