//! Pointer gesture ownership. The mirror terminal owns content anchors.
use shoal_model::PaneId;

#[derive(Debug)]
pub struct Selection {
    pub pane: PaneId,
    pub dragging: bool,
}
