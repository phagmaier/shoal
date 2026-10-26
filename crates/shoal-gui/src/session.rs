//! Connecting a window to `shoald` and choosing the session it shows (M1
//! spec §4.4, §5.3, §7.1): `hello` with the window's core and environment,
//! smart open or a named session, `window.show_session`, then the workspace
//! subscription and its first full state.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use shoal_client::{
    AutoStart, Client, Event, Options,
    model::{SessionId, Workspace},
    proto::{Call, ClientKind, HelloResult, Notification, WINDOW_ENV},
};

/// How long the first full workspace may take after subscribing.
const FIRST_STATE: Duration = Duration::from_secs(5);

/// Which session the window opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// `session.open_smart`, creating a session in `cwd` if one is needed.
    Smart { cwd: Option<PathBuf> },
    /// `shoal attach`: a session that must exist.
    Session(SessionId),
}

/// A window's connection, showing its session.
#[derive(Debug)]
pub struct Opened {
    pub client: Client,
    pub hello: HelloResult,
    pub session: SessionId,
    pub workspace: Workspace,
    /// Everything after the first full state, in order.
    pub events: Receiver<Event>,
    /// Notices that arrived before the first full state.
    pub early: Vec<Event>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Client(#[from] shoal_client::Error),
    #[error("unexpected answer from shoald: {0}")]
    Answer(String),
    #[error("shoald sent no workspace within {}s", FIRST_STATE.as_secs())]
    NoWorkspace,
}

/// The window's `hello`: its core, so the daemon can refuse attaches it
/// couldn't decode, and its session environment, reconnecting on loss.
pub fn options(socket: PathBuf, env: impl IntoIterator<Item = (String, String)>) -> Options {
    let mut options = Options::new(socket, ClientKind::Gui);
    options.core = Some(shoal_term::core_id().to_owned());
    options.env = window_env(env);
    options
}

/// The variables of `env` a window passes on (§4.2).
fn window_env(env: impl IntoIterator<Item = (String, String)>) -> BTreeMap<String, String> {
    env.into_iter()
        .filter(|(key, _)| WINDOW_ENV.contains(&key.as_str()))
        .collect()
}

/// Connect (starting the daemon if nothing is listening), choose the
/// session, show it and subscribe. Blocks until the first full workspace.
pub fn open(
    options: Options,
    auto_start: impl AutoStart,
    choice: Choice,
) -> Result<Opened, OpenError> {
    let connected = Client::connect(options, auto_start)?;
    let client = connected.client;
    let session = match choice {
        Choice::Smart { cwd } => {
            let result = client.request(Call::SessionOpenSmart { cwd })?;
            serde_json::from_value(result["session"].clone())
                .map_err(|error| OpenError::Answer(error.to_string()))?
        }
        Choice::Session(session) => session,
    };
    show(&client, session)?;
    client.subscribe_workspace()?;
    let (workspace, early) = first_state(&connected.events)?;
    Ok(Opened {
        client,
        hello: connected.hello,
        session,
        workspace,
        events: connected.events,
        early,
    })
}

/// `window.show_session`: counts this window on the session. Sent again
/// after each reconnect, because the daemon forgets a closed connection's.
pub fn show(client: &Client, session: SessionId) -> Result<(), shoal_client::Error> {
    client.request(show_call(session)).map(drop)
}

pub fn show_call(session: SessionId) -> Call {
    Call::WindowShowSession { session }
}

fn first_state(events: &Receiver<Event>) -> Result<(Workspace, Vec<Event>), OpenError> {
    let deadline = Instant::now() + FIRST_STATE;
    let mut early = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Event::Notification(Notification::WorkspaceChanged(state))) => {
                return Ok((state.workspace, early));
            }
            Ok(Event::Disconnected(error)) => return Err(error.into()),
            Ok(other) => early.push(other),
            Err(RecvTimeoutError::Timeout) => return Err(OpenError::NoWorkspace),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(shoal_client::Error::Stopped.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shoal_client::NoAutoStart;
    use shoal_testkit::{TempEnv, shoald};

    use super::*;

    fn open_in(env: &TempEnv, choice: Choice) -> Result<Opened, OpenError> {
        open(options(env.socket(), []), NoAutoStart, choice)
    }

    fn windows(workspace: &Workspace) -> Vec<(String, u32)> {
        workspace
            .sessions
            .iter()
            .map(|session| (session.name.clone(), session.windows))
            .collect()
    }

    #[test]
    fn windows_open_on_off_screen_sessions_first() {
        let env = TempEnv::new();
        let _daemon = env.spawn_daemon(shoald());
        let smart = || Choice::Smart {
            cwd: Some(env.home()),
        };

        let first = open_in(&env, smart()).unwrap();
        assert_eq!(windows(&first.workspace), [("main".to_owned(), 1)]);
        assert_eq!(first.workspace.sessions[0].id, first.session);

        // `main` is on screen, so the next window gets a new session.
        let second = open_in(&env, smart()).unwrap();
        assert_eq!(
            windows(&second.workspace),
            [("main".to_owned(), 1), ("2".to_owned(), 1)]
        );
        assert_ne!(second.session, first.session);

        // `shoal attach main` shows it in a second window.
        let attached = open_in(&env, Choice::Session(first.session)).unwrap();
        assert_eq!(attached.session, first.session);
        assert_eq!(
            windows(&attached.workspace),
            [("main".to_owned(), 2), ("2".to_owned(), 1)]
        );

        // Closing a window closes nothing; its session is off screen again
        // and the next smart open picks it.
        drop(second);
        let reopened = open_in(&env, smart()).unwrap();
        assert_eq!(reopened.workspace.sessions.len(), 2);
        assert_eq!(
            reopened.workspace.sessions[1].id, reopened.session,
            "session 2, not a third"
        );
    }

    #[test]
    fn a_missing_session_is_an_error() {
        let env = TempEnv::new();
        let _daemon = env.spawn_daemon(shoald());
        let error = open_in(&env, Choice::Session("s99".parse().unwrap())).unwrap_err();
        let OpenError::Client(error) = error else {
            panic!("{error:?}")
        };
        assert_eq!(error.code(), Some(1001), "not found: {error}");
    }

    #[test]
    fn hello_carries_the_core_and_only_allowlisted_variables() {
        let options = options(
            "/run/x.sock".into(),
            [
                ("WAYLAND_DISPLAY".to_owned(), "wayland-1".to_owned()),
                ("HOME".to_owned(), "/home/x".to_owned()),
                ("SSH_AUTH_SOCK".to_owned(), "/run/ssh".to_owned()),
            ],
        );
        assert_eq!(options.client, ClientKind::Gui);
        assert_eq!(options.core.as_deref(), Some(shoal_term::core_id()));
        assert!(options.reconnect);
        assert_eq!(
            options.env,
            BTreeMap::from([
                ("SSH_AUTH_SOCK".to_owned(), "/run/ssh".to_owned()),
                ("WAYLAND_DISPLAY".to_owned(), "wayland-1".to_owned()),
            ])
        );
    }
}
