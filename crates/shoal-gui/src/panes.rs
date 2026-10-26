//! The panes a window mirrors (M1 spec §5.5, §7.8): which ones stay
//! attached, their attach streams, and feeding each stream into its mirror
//! on the main thread.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{Context, Task};
use shoal_client::{
    Client, Detach, StreamItem,
    model::{PaneId, SessionId, Workspace},
};

use crate::{
    mirror::{Next, Setup, State},
    retention::Recent,
    terminal::Painter,
    view::ShoalWindow,
};

/// How long one turn may feed a pane before yielding, so a flood of output
/// still lets frames and input in.
const FEED_BUDGET: Duration = Duration::from_millis(4);

/// An attached pane.
pub struct Attached {
    pub state: State,
    pub painter: Painter,
    _stream: Stream,
}

/// A pane's stream: dropping it detaches and stops its feeding task.
struct Stream {
    _detach: Detach,
    _task: Task<()>,
}

pub struct Panes {
    setup: Setup,
    recent: Recent,
    attached: BTreeMap<PaneId, Attached>,
}

impl Panes {
    pub fn new(setup: Setup) -> Self {
        Self {
            setup,
            recent: Recent::default(),
            attached: BTreeMap::new(),
        }
    }

    pub fn get_mut(&mut self, pane: PaneId) -> Option<&mut Attached> {
        self.attached.get_mut(&pane)
    }

    /// Include retained hidden tabs and future snapshots without touching streams.
    pub fn apply_setup(&mut self, setup: Setup) {
        if self.setup != setup {
            self.setup = setup;
            for (pane, attached) in &mut self.attached {
                if let State::Live(mirror) = &mut attached.state
                    && let Err(error) = mirror.apply_setup(&setup)
                {
                    tracing::warn!(%pane, %error, "can't apply mirror settings");
                }
            }
        }
    }

    pub fn invalidate_rows(&mut self) {
        for attached in self.attached.values_mut() {
            if let State::Live(mirror) = &mut attached.state {
                mirror.invalidate_rows();
            }
            attached.painter = Painter::default();
        }
    }

    /// Drop streams and terminals at a connection boundary. The next sync
    /// attaches from fresh snapshots, including retained hidden tabs.
    pub fn clear(&mut self) {
        self.attached.clear();
    }

    /// Attach the panes of the recently viewed tabs, the session's active
    /// tab first, and detach the rest.
    pub fn sync(
        &mut self,
        workspace: &Workspace,
        session: SessionId,
        client: &Client,
        cx: &mut Context<ShoalWindow>,
    ) {
        if let Some(session) = workspace.sessions.iter().find(|s| s.id == session) {
            self.recent.view(workspace, session.active_tab);
        }
        let wanted = self.recent.panes(workspace);
        self.attached.retain(|pane, _| wanted.contains(pane));
        for pane in wanted {
            self.attached.entry(pane).or_insert_with(|| Attached {
                state: State::new(),
                painter: Painter::default(),
                _stream: attach(pane, client, cx),
            });
        }
    }

    /// Start over on a pane that shows a banner.
    pub fn retry(&mut self, pane: PaneId, client: &Client, cx: &mut Context<ShoalWindow>) {
        if let Some(attached) = self.attached.get_mut(&pane) {
            attached.state = State::new();
            attached._stream = attach(pane, client, cx);
        }
    }

    /// Apply `first` and whatever else is waiting, for at most
    /// [`FEED_BUDGET`]. Returns whether the pane needs a frame.
    fn feed(
        &mut self,
        pane: PaneId,
        first: StreamItem,
        more: &mut mpsc::UnboundedReceiver<StreamItem>,
        client: &Client,
        cx: &mut Context<ShoalWindow>,
    ) -> bool {
        let Some(attached) = self.attached.get_mut(&pane) else {
            return false;
        };
        let started = Instant::now();
        let mut item = first;
        loop {
            if let StreamItem::Failed(error) = &item
                && shoal_client::lifecycle::Mismatch::of_error(error).is_some()
            {
                cx.emit(error.clone());
                return true;
            }
            if attached.state.apply(item, &self.setup) == Next::Reattach {
                // The new stream replaces this one, which ends with this turn.
                attached._stream = attach(pane, client, cx);
                return true;
            }
            if started.elapsed() >= FEED_BUDGET {
                return true;
            }
            match more.try_recv() {
                Ok(next) => item = next,
                Err(_) => return true,
            }
        }
    }
}

/// Attach `pane` and feed its items into the window's mirror. A thread
/// waits on the client's channel; the feeding runs on the main thread,
/// where the mirror lives.
fn attach(pane: PaneId, client: &Client, cx: &mut Context<ShoalWindow>) -> Stream {
    let (items, detach) = client.attach(pane).split();
    let (sender, mut receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(format!("shoal-{pane}"))
        .spawn(move || {
            // Ends when the stream does, which dropping `detach` causes.
            while let Ok(item) = items.recv() {
                if sender.unbounded_send(item).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, %pane, "can't start a stream thread");
    }
    let task = cx.spawn(async move |this, cx| {
        while let Some(first) = receiver.next().await {
            let fed = this.update(cx, |view, cx| {
                let ShoalWindow { panes, client, .. } = view;
                // Mirrors of tabs out of view keep up without drawing.
                let redraw = panes.feed(pane, first, &mut receiver, client, cx);
                view.sync_terminal_focus();
                if redraw && view.shows(pane) {
                    cx.notify();
                }
            });
            if fed.is_err() {
                break;
            }
            YieldNow(false).await;
        }
    });
    Stream {
        _detach: detach,
        _task: task,
    }
}

/// Return to the executor once, so a frame that is due can be drawn.
struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
