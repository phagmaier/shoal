use super::*;
use crate::{Axis, TabId};

fn p(id: u64) -> PaneId {
    PaneId::new(id)
}

fn leaf(id: u64) -> Layout {
    Layout::Pane(p(id))
}

fn branch(id: u64, axis: Axis, first: Layout, second: Layout) -> Layout {
    Layout::Split {
        id: SplitId::new(id),
        axis,
        ratio: 0.5,
        first: Box::new(first),
        second: Box::new(second),
    }
}

fn tab(layout: Layout, focused: u64) -> Tab {
    Tab {
        id: TabId::new(1),
        name: None,
        layout,
        focused: p(focused),
        zoomed: None,
    }
}

#[test]
fn split_replaces_only_target_focuses_new_and_unzooms() {
    for (direction, axis) in [
        (SplitDirection::Right, Axis::Row),
        (SplitDirection::Down, Axis::Column),
    ] {
        let mut tab = tab(branch(2, Axis::Row, leaf(3), leaf(4)), 3);
        tab.zoomed = Some(p(3));
        split(&mut tab, p(4), direction, p(5), SplitId::new(6)).unwrap();
        assert_eq!(
            tab.layout,
            branch(2, Axis::Row, leaf(3), branch(6, axis, leaf(4), leaf(5)))
        );
        assert_eq!(tab.focused, p(5));
        assert_eq!(tab.zoomed, None);
    }
}

#[test]
fn split_accepts_single_pane_and_rejects_missing_or_colliding_ids_atomically() {
    let mut single = tab(leaf(3), 3);
    split(
        &mut single,
        p(3),
        SplitDirection::Down,
        p(4),
        SplitId::new(2),
    )
    .unwrap();
    assert_eq!(single.layout, branch(2, Axis::Column, leaf(3), leaf(4)));

    for (target, new, split_id, error) in [
        (99, 5, 6, LayoutError::PaneNotFound(p(99))),
        (3, 4, 6, LayoutError::IdInUse(4)),
        (3, 2, 6, LayoutError::IdInUse(2)),
        (3, 5, 3, LayoutError::IdInUse(3)),
        (3, 5, 1, LayoutError::IdInUse(1)),
        (3, 5, 5, LayoutError::IdInUse(5)),
    ] {
        let mut tab = single.clone();
        assert_eq!(
            split(
                &mut tab,
                p(target),
                SplitDirection::Right,
                p(new),
                SplitId::new(split_id)
            ),
            Err(error)
        );
        assert_eq!(tab, single);
    }
}

#[test]
fn close_collapses_parent_and_focuses_first_leaf_of_sibling() {
    let mut tab = tab(
        branch(
            2,
            Axis::Row,
            leaf(3),
            branch(4, Axis::Column, leaf(5), leaf(6)),
        ),
        3,
    );
    tab.zoomed = Some(p(3));
    assert_eq!(close(&mut tab, p(3)), Ok(Some(p(5))));
    assert_eq!(tab.layout, branch(4, Axis::Column, leaf(5), leaf(6)));
    assert_eq!(tab.focused, p(5));
    assert_eq!(tab.zoomed, None);
    assert_eq!(close(&mut tab, p(6)), Ok(Some(p(5))));
    assert_eq!(tab.layout, leaf(5));
}

#[test]
fn close_nested_focus_and_preserve_other_focus_and_zoom() {
    let original = tab(
        branch(
            2,
            Axis::Row,
            leaf(3),
            branch(4, Axis::Column, leaf(5), leaf(6)),
        ),
        6,
    );
    let mut nested = original.clone();
    assert_eq!(close(&mut nested, p(6)), Ok(Some(p(5))));
    assert_eq!(nested.layout, branch(2, Axis::Row, leaf(3), leaf(5)));
    let mut preserved = original;
    preserved.zoomed = Some(p(3));
    assert_eq!(close(&mut preserved, p(5)), Ok(Some(p(6))));
    assert_eq!(preserved.focused, p(6));
    assert_eq!(preserved.zoomed, Some(p(3)));
}

#[test]
fn close_last_pane_and_missing_pane_are_distinct() {
    let mut tab = tab(leaf(3), 3);
    tab.zoomed = Some(p(3));
    let before = tab.clone();
    assert_eq!(
        close(&mut tab, p(99)),
        Err(LayoutError::PaneNotFound(p(99)))
    );
    assert_eq!(tab, before);
    assert_eq!(close(&mut tab, p(3)), Ok(None));
    assert_eq!(tab.zoomed, None);
}

#[test]
fn zoom_focuses_switches_and_toggles_off_without_editing_layout() {
    let mut tab = tab(branch(2, Axis::Row, leaf(3), leaf(4)), 3);
    let layout = tab.layout.clone();
    toggle_zoom(&mut tab, p(4)).unwrap();
    assert_eq!((tab.focused, tab.zoomed), (p(4), Some(p(4))));
    toggle_zoom(&mut tab, p(3)).unwrap();
    assert_eq!((tab.focused, tab.zoomed), (p(3), Some(p(3))));
    toggle_zoom(&mut tab, p(3)).unwrap();
    assert_eq!(tab.zoomed, None);
    assert_eq!(tab.layout, layout);
    let before = tab.clone();
    assert_eq!(
        toggle_zoom(&mut tab, p(99)),
        Err(LayoutError::PaneNotFound(p(99)))
    );
    assert_eq!(tab, before);
}

#[test]
fn zoom_single_pane() {
    let mut tab = tab(leaf(3), 3);
    toggle_zoom(&mut tab, p(3)).unwrap();
    assert_eq!(tab.zoomed, Some(p(3)));
    toggle_zoom(&mut tab, p(3)).unwrap();
    assert_eq!(tab.zoomed, None);
}

fn ratio(layout: &Layout, target: u64) -> f32 {
    match layout {
        Layout::Split { id, ratio, .. } if id.get() == target => *ratio,
        Layout::Split { first, second, .. } => {
            let mut found = false;
            first.for_each_split(&mut |id| found |= id.get() == target);
            ratio(if found { first } else { second }, target)
        }
        Layout::Pane(_) => panic!("missing split {target}"),
    }
}

#[test]
fn set_ratio_targets_nested_split_and_clamps_at_both_bounds() {
    let mut layout = branch(
        2,
        Axis::Row,
        leaf(3),
        branch(4, Axis::Column, leaf(5), leaf(6)),
    );
    assert_eq!(set_ratio(&mut layout, SplitId::new(4), 0.75), Ok(true));
    assert_eq!(ratio(&layout, 4), 0.75);
    assert_eq!(ratio(&layout, 2), 0.5);
    for (input, expected) in [(f32::MIN, 0.1), (0.1, 0.1), (f32::MAX, 0.9), (0.9, 0.9)] {
        set_ratio(&mut layout, SplitId::new(2), input).unwrap();
        assert_eq!(ratio(&layout, 2), expected);
    }
    assert_eq!(set_ratio(&mut layout, SplitId::new(2), 0.9), Ok(false));
}

#[test]
fn set_ratio_rejects_nonfinite_values_and_missing_splits_atomically() {
    let original = branch(2, Axis::Row, leaf(3), leaf(4));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut layout = original.clone();
        assert_eq!(
            set_ratio(&mut layout, SplitId::new(2), value),
            Err(LayoutError::InvalidRatio)
        );
        assert_eq!(layout, original);
    }
    for mut layout in [original, leaf(3)] {
        let before = layout.clone();
        assert_eq!(
            set_ratio(&mut layout, SplitId::new(99), 0.5),
            Err(LayoutError::SplitNotFound(SplitId::new(99)))
        );
        assert_eq!(layout, before);
    }
}

#[test]
fn resize_selects_nearest_eligible_ancestor_and_grows_in_all_directions() {
    // p3 | (p5 / (p8 | p9)). For p8, left growth must skip its immediate
    // row divider, and up growth must skip that divider's axis altogether.
    let original = branch(
        2,
        Axis::Row,
        leaf(3),
        branch(
            4,
            Axis::Column,
            leaf(5),
            branch(7, Axis::Row, leaf(8), leaf(9)),
        ),
    );
    for (pane, direction, changed_split, expected) in [
        (8, Direction::Right, 7, 0.625),
        (8, Direction::Left, 2, 0.375),
        (8, Direction::Up, 4, 0.375),
        (5, Direction::Down, 4, 0.625),
        (9, Direction::Left, 7, 0.375),
        (3, Direction::Right, 2, 0.625),
    ] {
        let mut layout = original.clone();
        assert_eq!(resize(&mut layout, p(pane), direction, 0.125), Ok(true));
        for id in [2, 4, 7] {
            assert_eq!(
                ratio(&layout, id),
                if id == changed_split { expected } else { 0.5 }
            );
        }
    }
    for (pane, direction) in [
        (8, Direction::Down),
        (9, Direction::Right),
        (3, Direction::Left),
        (5, Direction::Up),
    ] {
        let mut layout = original.clone();
        assert_eq!(resize(&mut layout, p(pane), direction, 0.125), Ok(false));
        assert_eq!(layout, original);
    }
}

#[test]
fn resize_clamps_and_does_not_skip_a_clamped_ancestor() {
    let mut layout = branch(
        2,
        Axis::Row,
        branch(4, Axis::Row, leaf(5), leaf(6)),
        leaf(3),
    );
    assert_eq!(
        resize(&mut layout, p(5), Direction::Right, f32::MAX),
        Ok(true)
    );
    assert_eq!(ratio(&layout, 4), 0.9);
    let clamped = layout.clone();
    assert_eq!(
        resize(&mut layout, p(5), Direction::Right, 0.125),
        Ok(false)
    );
    assert_eq!(layout, clamped);
    assert_eq!(ratio(&layout, 2), 0.5);
    assert_eq!(
        resize(&mut layout, p(6), Direction::Left, f32::MAX),
        Ok(true)
    );
    assert_eq!(ratio(&layout, 4), 0.1);
}

#[test]
fn resize_zero_step_single_pane_and_invalid_arguments() {
    let original = branch(2, Axis::Column, leaf(3), leaf(4));
    let mut layout = original.clone();
    assert_eq!(resize(&mut layout, p(3), Direction::Down, 0.0), Ok(false));
    assert_eq!(layout, original);
    for step in [-0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            resize(&mut layout, p(3), Direction::Down, step),
            Err(LayoutError::InvalidStep)
        );
        assert_eq!(layout, original);
    }
    assert_eq!(
        resize(&mut layout, p(99), Direction::Down, 0.1),
        Err(LayoutError::PaneNotFound(p(99)))
    );
    assert_eq!(layout, original);
    for direction in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        assert_eq!(resize(&mut leaf(3), p(3), direction, 0.1), Ok(false));
    }
}
