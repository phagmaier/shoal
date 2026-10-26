//! Accepting authenticated connections and bounded pane shutdown (§4.1, §4.5).

use std::time::Duration;
use std::{io, path::Path, sync::Arc};

use shoal_config::XdgPaths;
use shoal_daemon::settings::{Settings, Update};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
    sync::{mpsc, watch},
    task::JoinSet,
};

use crate::{
    actor,
    connection::{self, Daemon},
    startup::Bound,
};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Stop accepting, save, end panes concurrently, close clients, then release
/// the socket and lock.
pub async fn serve(
    bound: Bound,
    daemon: Daemon,
    paths: XdgPaths,
    settings: Settings,
    updates: mpsc::UnboundedReceiver<Update>,
) -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    bound.listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(bound.listener)?;
    let daemon = Arc::new(daemon);
    let (shutdown, mut stopping) = watch::channel(false);
    let actor = actor::start(paths, settings, updates, shutdown);
    let mut connections = JoinSet::new();
    tracing::info!(socket = %bound.socket.display(), pid = daemon.pid, "shoald listening");
    let reason = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    match stream.peer_cred() {
                        Ok(peer) if peer.uid() == bound.uid => {
                            connections.spawn(connection::run(stream, daemon.clone(), actor.clone()));
                        }
                        Ok(peer) => tracing::warn!(uid = peer.uid(), "refused a connection from another user"),
                        Err(error) => tracing::warn!(%error, "refused a connection without peer credentials"),
                    }
                }
                // A persistent accept failure must not spin. This deadline
                // exists only after an error, never while idle.
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(ACCEPT_RETRY).await;
                }
            },
            _ = connections.join_next(), if !connections.is_empty() => {},
            _ = terminate.recv() => break "SIGTERM",
            _ = interrupt.recv() => break "SIGINT",
            _ = stopping.changed() => break "daemon.shutdown",
        }
    };
    tracing::info!(reason, "shoald stopping");
    drop(listener);
    actor.stop().await;
    // Let queued shutdown replies reach clients, but never let a nonreading
    // connection extend the daemon's bounded shutdown.
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();
    // Socket first: a daemon that takes the lock next binds a fresh socket,
    // which removing ours afterwards would delete.
    remove_socket(&bound.socket);
    drop(bound.lock);
    tracing::info!("shoald stopped");
    Ok(())
}

fn remove_socket(socket: &Path) {
    if let Err(error) = std::fs::remove_file(socket) {
        tracing::warn!(%error, socket = %socket.display(), "could not remove the socket");
    }
}
