//! Convert protocol cell counts to the pure model's fractional resize step.

use std::collections::BTreeMap;

use shoal_model::{Axis, Direction, Layout, Pane, PaneId};
use shoal_proto::{RpcError, error_code};

use super::{Actor, error, spawning::missing};

impl Actor {
    pub(super) fn resize_pane(
        &mut self,
        pane: PaneId,
        direction: Direction,
        cells: u64,
    ) -> Result<(), RpcError> {
        let (si, ti) = self
            .pane_location(pane)
            .ok_or_else(|| missing("pane", pane))?;
        if cells == 0 {
            return Ok(());
        }
        let tab = &mut self.workspace.sessions[si].tabs[ti];
        let Some(extent) = resize_extent(&tab.layout, pane, direction, &self.workspace.panes)
        else {
            return Ok(());
        };
        let step = (cells as f64 / extent.max(1.0)) as f32;
        if shoal_model::resize(&mut tab.layout, pane, direction, step)
            .map_err(|reason| error(error_code::BAD_PARAMS, reason.to_string()))?
        {
            self.changed();
        }
        Ok(())
    }
}

fn resize_extent(
    layout: &Layout,
    pane: PaneId,
    direction: Direction,
    panes: &BTreeMap<PaneId, Pane>,
) -> Option<f64> {
    let Layout::Split {
        axis,
        first,
        second,
        ..
    } = layout
    else {
        return None;
    };
    let in_first = first.contains_pane(pane);
    let child = if in_first { first } else { second };
    if let Some(extent) = resize_extent(child, pane, direction, panes) {
        return Some(extent);
    }
    let eligible = match (*axis, direction) {
        (Axis::Row, Direction::Right) | (Axis::Column, Direction::Down) => in_first,
        (Axis::Row, Direction::Left) | (Axis::Column, Direction::Up) => !in_first,
        _ => false,
    };
    if !eligible {
        return None;
    }
    let (cols, rows) = dimensions(layout, panes);
    Some(match axis {
        Axis::Row => cols,
        Axis::Column => rows,
    })
}

// Leaf grids, confirmed by their pane threads, are authoritative: add
// siblings on the split axis and take their maximum across it.
fn dimensions(layout: &Layout, panes: &BTreeMap<PaneId, Pane>) -> (f64, f64) {
    match layout {
        Layout::Pane(id) => {
            let size = panes[id].size;
            (f64::from(size.cols), f64::from(size.rows))
        }
        Layout::Split {
            axis,
            first,
            second,
            ..
        } => {
            let (ac, ar) = dimensions(first, panes);
            let (bc, br) = dimensions(second, panes);
            match axis {
                Axis::Row => (ac + bc, ar.max(br)),
                Axis::Column => (ac.max(bc), ar + br),
            }
        }
    }
}
