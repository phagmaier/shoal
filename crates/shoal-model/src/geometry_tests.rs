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
    // The exact stored f32 values are 1/4, 13_421_773/134_217_728
    // and 7_549_747/8_388_608. The expected integer shares are their
    // products rounded down, including the case where f64 rounds up first.
    for (ratio, extent, first, second) in [
        (0.25, u32::MAX, 1_073_741_823, 3_221_225_472),
        (0.1, u32::MAX, 429_496_735, 3_865_470_560),
        (0.9, 4_290_772_997, 3_861_695_594, 429_077_403),
    ] {
        for axis in [Axis::Row, Axis::Column] {
            let layout = branch(1, axis, ratio, Layout::Pane(p(2)), Layout::Pane(p(3)));
            let (area, a, b) = match axis {
                Axis::Row => (
                    r(0, 0, extent, 1),
                    r(0, 0, first, 1),
                    r(first, 0, second, 1),
                ),
                Axis::Column => (
                    r(0, 0, 1, extent),
                    r(0, 0, 1, first),
                    r(0, first, 1, second),
                ),
            };
            assert_eq!(
                rects(&layout, area, 0),
                Ok(BTreeMap::from([(p(2), a), (p(3), b)]))
            );
        }
    }
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
fn dividers_sit_in_the_gaps_between_the_rects() {
    // The same layout as above: the row split's gap is x 56..59, and the
    // column split's, inside the second child, is y 36..39.
    let layout = branch(
        1,
        Axis::Row,
        0.5,
        Layout::Pane(p(2)),
        branch(3, Axis::Column, 0.5, Layout::Pane(p(4)), Layout::Pane(p(5))),
    );
    let found = dividers(&layout, r(7, 11, 102, 54), 3).unwrap();
    assert_eq!(
        found,
        vec![
            Divider {
                split: SplitId::new(1),
                axis: Axis::Row,
                ratio: 0.5,
                area: r(7, 11, 102, 54),
                gap: r(56, 11, 3, 54),
            },
            Divider {
                split: SplitId::new(3),
                axis: Axis::Column,
                ratio: 0.5,
                area: r(59, 11, 50, 54),
                gap: r(59, 36, 50, 3),
            },
        ]
    );
    assert_eq!(found[0].usable(), 99);
    assert_eq!(found[1].usable(), 51);
    // Without a gap the divider is empty, at the first child's edge.
    let flat = dividers(&layout, r(0, 0, 100, 50), 0).unwrap();
    assert_eq!(flat[0].gap, r(50, 0, 0, 50));
    assert_eq!(flat[0].usable(), 100);
    assert!(
        dividers(&Layout::Pane(p(2)), r(0, 0, 10, 10), 3)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        dividers(&layout, r(u32::MAX, 0, 1, 1), 0),
        Err(LayoutError::AreaOverflow)
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
