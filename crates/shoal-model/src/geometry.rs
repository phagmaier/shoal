//! Integer pixel geometry and directional pane navigation (M1 spec §3).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use crate::{Axis, Direction, Layout, LayoutError, PaneId, SplitId};

/// A half-open rectangle in tab-local pixels. Zero extents are allowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(self) -> u64 {
        u64::from(self.x) + u64::from(self.width)
    }

    fn bottom(self) -> u64 {
        u64::from(self.y) + u64::from(self.height)
    }

    fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Computes the underlying layout even when the caller displays a zoomed pane.
/// Gaps are capped to each split's extent; the second child receives rounding
/// remainders. Rejects areas whose right or bottom coordinate overflows `u32`.
/// Requires a valid layout, as checked by [`crate::validate`].
pub fn rects(layout: &Layout, area: Rect, gap: u32) -> Result<BTreeMap<PaneId, Rect>, LayoutError> {
    if area.x.checked_add(area.width).is_none() || area.y.checked_add(area.height).is_none() {
        return Err(LayoutError::AreaOverflow);
    }
    let mut result = BTreeMap::new();
    collect_rects(layout, area, gap, &mut result);
    Ok(result)
}

/// The gap between a split's children: the drag target that moves its ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Divider {
    pub split: SplitId,
    pub axis: Axis,
    pub ratio: f32,
    /// The split's whole area, both children and the gap.
    pub area: Rect,
    /// Between the children; empty when the gap is 0.
    pub gap: Rect,
}

impl Divider {
    /// The extent along the axis that the ratio shares out: the area less
    /// the gap.
    pub fn usable(&self) -> u32 {
        match self.axis {
            Axis::Row => self.area.width - self.gap.width,
            Axis::Column => self.area.height - self.gap.height,
        }
    }
}

/// Every split's divider, parent before children, laid out as [`rects`]
/// lays out the panes.
pub fn dividers(layout: &Layout, area: Rect, gap: u32) -> Result<Vec<Divider>, LayoutError> {
    if area.x.checked_add(area.width).is_none() || area.y.checked_add(area.height).is_none() {
        return Err(LayoutError::AreaOverflow);
    }
    let mut result = Vec::new();
    collect_dividers(layout, area, gap, &mut result);
    Ok(result)
}

fn collect_rects(layout: &Layout, area: Rect, gap: u32, result: &mut BTreeMap<PaneId, Rect>) {
    match layout {
        Layout::Pane(pane) => {
            result.insert(*pane, area);
        }
        Layout::Split {
            axis,
            ratio,
            first,
            second,
            ..
        } => {
            let [first_area, _, second_area] = divide(*axis, *ratio, area, gap);
            collect_rects(first, first_area, gap, result);
            collect_rects(second, second_area, gap, result);
        }
    }
}

fn collect_dividers(layout: &Layout, area: Rect, gap: u32, result: &mut Vec<Divider>) {
    if let Layout::Split {
        id,
        axis,
        ratio,
        first,
        second,
    } = layout
    {
        let [first_area, gap_area, second_area] = divide(*axis, *ratio, area, gap);
        result.push(Divider {
            split: *id,
            axis: *axis,
            ratio: *ratio,
            area,
            gap: gap_area,
        });
        collect_dividers(first, first_area, gap, result);
        collect_dividers(second, second_area, gap, result);
    }
}

/// A split's first child, gap and second child.
fn divide(axis: Axis, ratio: f32, area: Rect, gap: u32) -> [Rect; 3] {
    let extent = match axis {
        Axis::Row => area.width,
        Axis::Column => area.height,
    };
    let local_gap = gap.min(extent);
    let usable = extent - local_gap;
    // Every allowed f32 ratio (0.1..=0.9) is an exact multiple of 2^-27.
    // Scaling by that power of two is exact; the integer product then
    // fits in 59 bits and division floors without floating-point rounding.
    const RATIO_SCALE: u64 = 1 << 27;
    let share = (f64::from(ratio) * RATIO_SCALE as f64) as u64;
    let first_extent = (u64::from(usable) * share / RATIO_SCALE) as u32;
    let second_extent = usable - first_extent;
    match axis {
        Axis::Row => [
            Rect {
                width: first_extent,
                ..area
            },
            Rect {
                x: area.x + first_extent,
                width: local_gap,
                ..area
            },
            Rect {
                x: area.x + first_extent + local_gap,
                width: second_extent,
                ..area
            },
        ],
        Axis::Column => [
            Rect {
                height: first_extent,
                ..area
            },
            Rect {
                y: area.y + first_extent,
                height: local_gap,
                ..area
            },
            Rect {
                y: area.y + first_extent + local_gap,
                height: second_extent,
                ..area
            },
        ],
    }
}

/// Finds the closest facing edge with positive perpendicular overlap. Ties favor
/// greater overlap, then the topmost, leftmost pane. No wraparound.
pub fn neighbor(
    rects: &BTreeMap<PaneId, Rect>,
    pane: PaneId,
    direction: Direction,
) -> Option<PaneId> {
    let source = *rects.get(&pane)?;
    if source.is_empty() {
        return None;
    }
    rects
        .iter()
        .filter(|(id, _)| **id != pane)
        .filter_map(|(&id, &candidate)| {
            candidate_score(source, candidate, direction).map(|score| ((score, id), id))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, id)| id)
}

fn candidate_score(
    source: Rect,
    candidate: Rect,
    direction: Direction,
) -> Option<(u64, Reverse<u64>, u32, u32)> {
    if candidate.is_empty() {
        return None;
    }
    let distance = match direction {
        Direction::Left => u64::from(source.x).checked_sub(candidate.right())?,
        Direction::Right => u64::from(candidate.x).checked_sub(source.right())?,
        Direction::Up => u64::from(source.y).checked_sub(candidate.bottom())?,
        Direction::Down => u64::from(candidate.y).checked_sub(source.bottom())?,
    };
    let (start, end) = match direction {
        Direction::Left | Direction::Right => (
            u64::from(source.y.max(candidate.y)),
            source.bottom().min(candidate.bottom()),
        ),
        Direction::Up | Direction::Down => (
            u64::from(source.x.max(candidate.x)),
            source.right().min(candidate.right()),
        ),
    };
    let overlap = end.checked_sub(start)?;
    (overlap > 0).then_some((distance, Reverse(overlap), candidate.y, candidate.x))
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
