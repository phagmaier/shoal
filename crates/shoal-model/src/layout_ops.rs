//! Pure tab and layout edits (M1 spec §3).

use serde::{Deserialize, Serialize};

use crate::{Axis, Layout, PaneId, SplitId, Tab};

/// Directions in pane geometry, without wraparound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// A new pane is placed after the old one on the chosen axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Right,
    Down,
}

/// Invalid operations leave their input unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("pane {0} is not in the layout")]
    PaneNotFound(PaneId),
    #[error("split {0} is not in the layout")]
    SplitNotFound(SplitId),
    #[error("id {0} is already in use")]
    IdInUse(u64),
    #[error("ratio must be finite")]
    InvalidRatio,
    #[error("resize step must be finite and nonnegative")]
    InvalidStep,
    #[error("area coordinates overflow")]
    AreaOverflow,
}

/// Splits a leaf 50/50, focuses `new`, and clears zoom. The caller allocates
/// globally fresh ids and inserts the new pane record into the workspace.
pub fn split(
    tab: &mut Tab,
    pane: PaneId,
    direction: SplitDirection,
    new: PaneId,
    split_id: SplitId,
) -> Result<(), LayoutError> {
    require_pane(&tab.layout, pane)?;
    for number in [new.get(), split_id.get()] {
        let mut used = number == tab.id.get();
        tab.layout
            .for_each_pane(&mut |id| used |= id.get() == number);
        tab.layout
            .for_each_split(&mut |id| used |= id.get() == number);
        if used {
            return Err(LayoutError::IdInUse(number));
        }
    }
    if new.get() == split_id.get() {
        return Err(LayoutError::IdInUse(new.get()));
    }
    let target = pane_mut(&mut tab.layout, pane)
        .expect("invariant: target pane was checked before splitting");
    *target = Layout::Split {
        id: split_id,
        axis: match direction {
            SplitDirection::Right => Axis::Row,
            SplitDirection::Down => Axis::Column,
        },
        ratio: 0.5,
        first: Box::new(Layout::Pane(pane)),
        second: Box::new(Layout::Pane(new)),
    };
    tab.focused = new;
    tab.zoomed = None;
    Ok(())
}

/// Removes a pane and collapses its parent. Returns the surviving focus, or
/// `None` for the last pane; the caller must then remove the tab immediately.
pub fn close(tab: &mut Tab, pane: PaneId) -> Result<Option<PaneId>, LayoutError> {
    require_pane(&tab.layout, pane)?;
    if tab.zoomed == Some(pane) {
        tab.zoomed = None;
    }
    if matches!(tab.layout, Layout::Pane(_)) {
        return Ok(None);
    }
    let sibling_focus =
        remove_leaf(&mut tab.layout, pane).expect("invariant: pane exists in a split tree");
    if tab.focused == pane {
        tab.focused = sibling_focus;
    }
    Ok(Some(tab.focused))
}

/// Focuses and toggles zoom on a pane, including a single-pane tab.
pub fn toggle_zoom(tab: &mut Tab, pane: PaneId) -> Result<(), LayoutError> {
    require_pane(&tab.layout, pane)?;
    tab.focused = pane;
    tab.zoomed = (tab.zoomed != Some(pane)).then_some(pane);
    Ok(())
}

fn require_pane(layout: &Layout, pane: PaneId) -> Result<(), LayoutError> {
    if layout.contains_pane(pane) {
        Ok(())
    } else {
        Err(LayoutError::PaneNotFound(pane))
    }
}

fn pane_mut(layout: &mut Layout, pane: PaneId) -> Option<&mut Layout> {
    match layout {
        Layout::Pane(id) if *id == pane => Some(layout),
        Layout::Split { first, second, .. } => {
            pane_mut(first, pane).or_else(|| pane_mut(second, pane))
        }
        _ => None,
    }
}

fn first_pane(layout: &Layout) -> PaneId {
    match layout {
        Layout::Pane(pane) => *pane,
        Layout::Split { first, .. } => first_pane(first),
    }
}

/// A successful removal replaces exactly the immediate parent with its sibling.
fn remove_leaf(layout: &mut Layout, pane: PaneId) -> Option<PaneId> {
    let Layout::Split { first, second, .. } = layout else {
        return None;
    };
    let sibling = if matches!(**first, Layout::Pane(id) if id == pane) {
        second
    } else if matches!(**second, Layout::Pane(id) if id == pane) {
        first
    } else {
        return remove_leaf(first, pane).or_else(|| remove_leaf(second, pane));
    };
    let focus = first_pane(sibling);
    *layout = std::mem::replace(sibling.as_mut(), Layout::Pane(pane));
    Some(focus)
}

/// Sets a finite ratio, clamped to `Layout::RATIO_RANGE`. Returns whether it changed.
pub fn set_ratio(layout: &mut Layout, split_id: SplitId, ratio: f32) -> Result<bool, LayoutError> {
    if !ratio.is_finite() {
        return Err(LayoutError::InvalidRatio);
    }
    let target = split_ratio_mut(layout, split_id).ok_or(LayoutError::SplitNotFound(split_id))?;
    Ok(update_ratio(target, ratio))
}

fn split_ratio_mut(layout: &mut Layout, split: SplitId) -> Option<&mut f32> {
    match layout {
        Layout::Split { id, ratio, .. } if *id == split => Some(ratio),
        Layout::Split { first, second, .. } => {
            split_ratio_mut(first, split).or_else(|| split_ratio_mut(second, split))
        }
        Layout::Pane(_) => None,
    }
}

fn update_ratio(target: &mut f32, value: f32) -> bool {
    let clamped = value.clamp(*Layout::RATIO_RANGE.start(), *Layout::RATIO_RANGE.end());
    let changed = *target != clamped;
    *target = clamped;
    changed
}

/// Grows a pane toward `direction` by a fraction of its nearest eligible ancestor's
/// usable extent. Returns whether the selected divider moved, after clamping.
pub fn resize(
    layout: &mut Layout,
    pane: PaneId,
    direction: Direction,
    step: f32,
) -> Result<bool, LayoutError> {
    if !step.is_finite() || step < 0.0 {
        return Err(LayoutError::InvalidStep);
    }
    require_pane(layout, pane)?;
    Ok(resize_ancestor(layout, pane, direction, step).unwrap_or(false))
}

/// `Some(false)` means the nearest eligible divider was already clamped; do not
/// fall through to a more distant ancestor in that case.
fn resize_ancestor(
    layout: &mut Layout,
    pane: PaneId,
    direction: Direction,
    step: f32,
) -> Option<bool> {
    let Layout::Split {
        axis,
        ratio,
        first,
        second,
        ..
    } = layout
    else {
        return None;
    };
    let in_first = first.contains_pane(pane);
    let child = if in_first { first } else { second };
    if let Some(changed) = resize_ancestor(child, pane, direction, step) {
        return Some(changed);
    }
    let eligible = match (*axis, direction) {
        (Axis::Row, Direction::Right) | (Axis::Column, Direction::Down) => in_first,
        (Axis::Row, Direction::Left) | (Axis::Column, Direction::Up) => !in_first,
        _ => false,
    };
    if !eligible {
        return None;
    }
    let value = if in_first {
        *ratio + step
    } else {
        *ratio - step
    };
    Some(update_ratio(ratio, value))
}

#[cfg(test)]
#[path = "layout_ops_tests.rs"]
mod tests;
