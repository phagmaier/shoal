//! The counting allocator behind restored terminals (M1 spec §5.5).
//!
//! libghostty keeps using a terminal's allocator for the terminal's whole life,
//! but the cap applies only while a snapshot decodes. Terminals never leave
//! their thread, so the budget is per thread: [`Budget::start`] arms it for one
//! synchronous decode and dropping the guard disarms it, even on an error.

use std::{alloc::Layout, cell::Cell, ptr::NonNull};

use allocator_api2::alloc::{AllocError, Allocator, Global};

pub(crate) type GhosttyAllocator = libghostty_vt::alloc::Allocator<'static>;

/// Forwards to the global allocator, counting live bytes while a budget is armed.
struct Counting;

static COUNTING: Counting = Counting;

#[derive(Clone, Copy)]
struct State {
    live: usize,
    peak: usize,
    cap: usize,
    refused: bool,
}

thread_local! {
    static STATE: Cell<Option<State>> = const { Cell::new(None) };
    // libghostty may hold the allocator's address for a terminal's lifetime,
    // so each thread gets one that is never freed.
    static ALLOCATOR: &'static GhosttyAllocator =
        Box::leak(Box::new(libghostty_vt::alloc::Allocator::from(&COUNTING)));
}

/// This thread's counting allocator, for terminals that decode snapshots.
pub(crate) fn allocator() -> &'static GhosttyAllocator {
    ALLOCATOR.with(|allocator| *allocator)
}

/// An armed allocation cap for one decode on this thread.
pub(crate) struct Budget(());

/// What one decode allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) peak: usize,
    pub(crate) refused: bool,
}

impl Budget {
    pub(crate) fn start(cap: usize) -> Self {
        STATE.with(|state| {
            state.set(Some(State {
                live: 0,
                peak: 0,
                cap,
                refused: false,
            }))
        });
        Self(())
    }

    pub(crate) fn usage(&self) -> Usage {
        let state = STATE
            .with(Cell::get)
            .expect("invariant: a live budget keeps its state armed");
        Usage {
            peak: state.peak,
            refused: state.refused,
        }
    }
}

impl Drop for Budget {
    fn drop(&mut self) {
        STATE.with(|state| state.set(None));
    }
}

fn reserve(size: usize) -> Result<(), AllocError> {
    STATE.with(|cell| {
        let Some(mut state) = cell.get() else {
            return Ok(());
        };
        let live = state
            .live
            .checked_add(size)
            .filter(|&live| live <= state.cap);
        let result = match live {
            Some(live) => {
                state.live = live;
                state.peak = state.peak.max(live);
                Ok(())
            }
            None => {
                state.refused = true;
                Err(AllocError)
            }
        };
        cell.set(Some(state));
        result
    })
}

fn release(size: usize) {
    STATE.with(|cell| {
        if let Some(mut state) = cell.get() {
            state.live = state.live.saturating_sub(size);
            cell.set(Some(state));
        }
    });
}

// SAFETY: every block comes from and returns to `Global` with the caller's
// layout; the counting has no effect on the memory itself.
unsafe impl Allocator for Counting {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        reserve(layout.size())?;
        Global
            .allocate(layout)
            .inspect_err(|_| release(layout.size()))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        release(layout.size());
        // SAFETY: the caller passes a block `allocate` returned with `layout`.
        unsafe { Global.deallocate(ptr, layout) }
    }
}
