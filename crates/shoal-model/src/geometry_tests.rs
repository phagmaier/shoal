use super::*;
use crate::{Axis, SplitId};

fn r(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn p(id: u64) -> PaneId {
    PaneId::new(id)
}

fn branch(id: u64, axis: Axis, ratio: f32, first: Layout, second: Layout) -> Layout {
    Layout::Split {
        id: SplitId::new(id),
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

#[test]
fn rects_single_pane_fills_offset_area_without_a_gap() {
    let area = r(7, 11, 101, 53);
    assert_eq!(
        rects(&Layout::Pane(p(1)), area, 100),
        Ok(BTreeMap::from([(p(1), area)]))
    );
}

#[test]
fn rects_nested_splits_floor_first_share_and_give_remainder_to_second() {
    let layout = branch(
        1,
        Axis::Row,
        0.5,
        Layout::Pane(p(2)),
        branch(3, Axis::Column, 0.5, Layout::Pane(p(4)), Layout::Pane(p(5))),
    );
    assert_eq!(
        rects(&layout, r(7, 11, 102, 54), 3),
        Ok(BTreeMap::from([
            (p(2), r(7, 11, 49, 54)),
            (p(4), r(59, 11, 50, 25)),
            (p(5), r(59, 39, 50, 26)),
        ]))
    );
}

#[test]
fn rects_respect_ratios_and_large_pixel_extents() {
    let layout = branch(1, Axis::Row, 0.25, Layout::Pane(p(2)), Layout::Pane(p(3)));
    assert_eq!(
        rects(&layout, r(0, 0, u32::MAX, 1), 0),
        Ok(BTreeMap::from([
            (p(2), r(0, 0, 1_073_741_823, 1)),
            (p(3), r(1_073_741_823, 0, 3_221_225_472, 1)),
        ]))
    );
}

#[test]
fn rects_cap_gaps_for_tiny_and_zero_areas() {
    for (axis, area, first, second) in [
        (Axis::Row, r(10, 20, 2, 3), r(10, 20, 0, 3), r(12, 20, 0, 3)),
        (
            Axis::Column,
            r(10, 20, 3, 2),
            r(10, 20, 3, 0),
            r(10, 22, 3, 0),
        ),
        (Axis::Row, r(10, 20, 0, 0), r(10, 20, 0, 0), r(10, 20, 0, 0)),
    ] {
        let layout = branch(1, axis, 0.5, Layout::Pane(p(2)), Layout::Pane(p(3)));
        assert_eq!(
            rects(&layout, area, u32::MAX),
            Ok(BTreeMap::from([(p(2), first), (p(3), second)]))
        );
    }
}

#[test]
fn rects_reject_overflowing_area_coordinates() {
    for area in [r(u32::MAX, 0, 1, 0), r(0, u32::MAX, 0, 1)] {
        assert_eq!(
            rects(&Layout::Pane(p(1)), area, 0),
            Err(LayoutError::AreaOverflow)
        );
    }
    let area = r(u32::MAX, u32::MAX, 0, 0);
    assert_eq!(
        rects(&Layout::Pane(p(1)), area, 0),
        Ok(BTreeMap::from([(p(1), area)]))
    );
}

#[test]
fn rects_cap_each_gap_independently_on_alternating_axes() {
    let layout = branch(
        1,
        Axis::Row,
        0.5,
        Layout::Pane(p(2)),
        branch(3, Axis::Column, 0.5, Layout::Pane(p(4)), Layout::Pane(p(5))),
    );
    assert_eq!(
        rects(&layout, r(10, 20, 2, 24), 6),
        Ok(BTreeMap::from([
            (p(2), r(10, 20, 0, 24)),
            (p(4), r(12, 20, 0, 9)),
            (p(5), r(12, 35, 0, 9)),
        ]))
    );
}

#[test]
fn neighbor_finds_all_four_directions_and_does_not_wrap() {
    let rects = BTreeMap::from([
        (p(1), r(10, 10, 10, 10)),
        (p(2), r(0, 10, 5, 10)),
        (p(3), r(25, 10, 5, 10)),
        (p(4), r(10, 0, 10, 5)),
        (p(5), r(10, 25, 10, 5)),
    ]);
    for (direction, id) in [
        (Direction::Left, 2),
        (Direction::Right, 3),
        (Direction::Up, 4),
        (Direction::Down, 5),
    ] {
        assert_eq!(neighbor(&rects, p(1), direction), Some(p(id)));
    }
    assert_eq!(neighbor(&rects, p(2), Direction::Left), None);
    assert_eq!(neighbor(&rects, p(99), Direction::Right), None);
    assert_eq!(
        neighbor(
            &BTreeMap::from([(p(1), r(0, 0, 10, 10))]),
            p(1),
            Direction::Right
        ),
        None
    );
}

#[test]
fn neighbor_uses_edge_distance_before_overlap_then_position() {
    let mut rects = BTreeMap::from([
        (p(1), r(0, 10, 10, 30)),
        (p(2), r(20, 10, 10, 30)), // More overlap but farther away.
        (p(3), r(12, 10, 10, 5)),
        (p(4), r(12, 25, 10, 10)), // Same distance, more overlap.
        (p(5), r(12, 15, 10, 10)), // Same distance/overlap, topmost.
    ]);
    assert_eq!(neighbor(&rects, p(1), Direction::Right), Some(p(5)));
    rects.remove(&p(5));
    assert_eq!(neighbor(&rects, p(1), Direction::Right), Some(p(4)));
    rects.remove(&p(4));
    assert_eq!(neighbor(&rects, p(1), Direction::Right), Some(p(3)));
}

#[test]
fn neighbor_tie_prefers_leftmost_and_measures_facing_edges() {
    let rects = BTreeMap::from([
        (p(1), r(0, 100, 50, 10)),
        (p(2), r(30, 20, 10, 70)), // Taller: its bottom is closer, despite its center.
        (p(3), r(0, 80, 10, 5)),
        (p(4), r(10, 20, 10, 70)), // Same distance, overlap and top; farther left.
    ]);
    assert_eq!(neighbor(&rects, p(1), Direction::Up), Some(p(4)));
}

#[test]
fn neighbor_requires_positive_overlap_and_nonempty_rectangles() {
    let rects = BTreeMap::from([
        (p(1), r(0, 0, 10, 10)),
        (p(2), r(10, 10, 10, 10)), // Only touches a corner.
        (p(3), r(5, 0, 10, 10)),   // Overlaps instead of facing the right edge.
        (p(4), r(10, 0, 0, 10)),
        (p(5), r(10, 0, 10, 0)),
    ]);
    assert_eq!(neighbor(&rects, p(1), Direction::Right), None);
    assert_eq!(neighbor(&rects, p(4), Direction::Right), None);
    assert_eq!(neighbor(&rects, p(5), Direction::Down), None);
    let adjacent = BTreeMap::from([(p(1), r(0, 0, 10, 10)), (p(2), r(10, 0, 10, 10))]);
    assert_eq!(neighbor(&adjacent, p(1), Direction::Right), Some(p(2)));
}
