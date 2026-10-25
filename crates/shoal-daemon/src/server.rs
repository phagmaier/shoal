//! Accepting connections until `SIGTERM` or `SIGINT` (M1 spec §4.1, §4.5).

use std::time::Duration;
use std::{io, path::Path, sync::Arc};

use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};

use crate::{
    connection::{self, Daemon},
    startup::Bound,
};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Serve until a stop signal, then remove the socket. Shutdown's persisting and
/// pane ending (§4.5) arrive with the workspace and panes.
pub async fn serve(bound: Bound, daemon: Daemon) -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    bound.listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(bound.listener)?;
    let daemon = Arc::new(daemon);
    tracing::info!(socket = %bound.socket.display(), pid = daemon.pid, "shoald listening");
    let signal = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    match stream.peer_cred() {
                        Ok(peer) if peer.uid() == bound.uid => {
                            tokio::spawn(connection::run(stream, daemon.clone()));
                        }
                        Ok(peer) => tracing::warn!(uid = peer.uid(), "refused a connection from another user"),
                        Err(error) => tracing::warn!(%error, "refused a connection without peer credentials"),
                    }
                }
                // Such as running out of file descriptors. The pause stops a
                // lasting error from spinning; it never runs while idle.
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(ACCEPT_RETRY).await;
                }
            },
            _ = terminate.recv() => break "SIGTERM",
            _ = interrupt.recv() => break "SIGINT",
        }
    };
    tracing::info!(signal, "shoald stopping");
    drop(listener);
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
