use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{ConfigFile, LoadedConfig};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("config watcher: {0}")]
    Notify(#[from] notify::Error),
    #[error("config watcher: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Config(#[from] crate::ConfigError),
}

/// A reload result always carries the last good config, even on an error.
#[derive(Debug)]
pub struct ConfigUpdate {
    pub loaded: LoadedConfig,
    pub error: Option<WatchError>,
}

/// Native config watching. Callbacks run on the worker thread and should forward
/// updates promptly. Dropping the watcher stops and joins that worker.
pub struct ConfigWatcher {
    control: Sender<Control>,
    worker: Option<JoinHandle<()>>,
}

enum Control {
    Event(notify::Result<Event>),
    Stop,
}

const DEBOUNCE: Duration = Duration::from_millis(100);
// Continuous edits must not postpone applying changes indefinitely.
const MAX_BURST: Duration = Duration::from_millis(750);

impl ConfigWatcher {
    /// Install watches before the initial read. Missing files/directories do not
    /// get created. The returned update reports startup errors against defaults.
    pub fn new(
        path: &Path,
        on_change: impl FnMut(ConfigUpdate) + Send + 'static,
    ) -> Result<(Self, ConfigUpdate), WatchError> {
        let (control, receiver) = mpsc::channel();
        let events = control.clone();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = events.send(Control::Event(event));
        })?;
        let mut paths = WatchPaths {
            file: std::path::absolute(path)?,
            target: None,
            active: BTreeSet::new(),
        };
        paths.refresh(&mut watcher)?;
        let mut file = ConfigFile::new(&paths.file);
        let initial = reload(&mut file, None);
        let worker = thread::Builder::new()
            .name("shoal-config".into())
            .spawn(move || {
                run(watcher, paths, file, receiver, on_change);
            })?;
        Ok((
            Self {
                control,
                worker: Some(worker),
            },
            initial,
        ))
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct WatchPaths {
    file: PathBuf,
    target: Option<PathBuf>,
    active: BTreeSet<PathBuf>,
}

impl WatchPaths {
    fn relevant(&self, event: &Event) -> bool {
        if event.need_rescan() {
            return true;
        }
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            return false;
        }
        event.paths.iter().any(|path| {
            self.file.starts_with(path)
                || self
                    .target
                    .as_ref()
                    .is_some_and(|target| target.starts_with(path))
        })
    }

    fn refresh(&mut self, watcher: &mut RecommendedWatcher) -> Result<(), WatchError> {
        self.target = self.file.canonicalize().ok().or_else(|| {
            std::fs::read_link(&self.file).ok().map(|target| {
                if target.is_absolute() {
                    target
                } else {
                    self.file.parent().unwrap_or(Path::new("/")).join(target)
                }
            })
        });
        // Nonrecursive ancestor watches catch future directories and replacements
        // without recursively watching the user's home. Rearm even identical
        // paths: an atomic directory replacement may have invalidated the inode.
        let desired: BTreeSet<_> = std::iter::once(&self.file)
            .chain(self.target.iter())
            .filter_map(|file| file.parent())
            .flat_map(Path::ancestors)
            .filter(|path| path.is_dir())
            .map(Path::to_path_buf)
            .collect();
        let mut error = None;
        for path in std::mem::take(&mut self.active) {
            if let Err(source) = watcher.unwatch(&path)
                && !matches!(source.kind, notify::ErrorKind::WatchNotFound)
            {
                error = Some(WatchError::Notify(source));
            }
        }
        for path in desired {
            match watcher.watch(&path, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.active.insert(path);
                }
                Err(source) => {
                    error = Some(WatchError::Notify(source));
                }
            }
        }
        error.map_or(Ok(()), Err)
    }
}

fn run(
    mut watcher: RecommendedWatcher,
    mut paths: WatchPaths,
    mut file: ConfigFile,
    receiver: Receiver<Control>,
    mut on_change: impl FnMut(ConfigUpdate),
) {
    loop {
        match receiver.recv() {
            Ok(Control::Stop) | Err(_) => return,
            Ok(Control::Event(Err(error))) => {
                on_change(ConfigUpdate {
                    loaded: file.current().clone(),
                    error: Some(error.into()),
                });
                continue;
            }
            Ok(Control::Event(Ok(event))) if !paths.relevant(&event) => continue,
            Ok(Control::Event(Ok(_))) => {}
        }
        let limit = Instant::now() + MAX_BURST;
        let mut quiet_at = Instant::now() + DEBOUNCE;
        let mut error = None;
        loop {
            if Instant::now() >= quiet_at {
                break;
            }
            match receiver.recv_timeout(quiet_at.saturating_duration_since(Instant::now())) {
                Ok(Control::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
                Ok(Control::Event(Err(source))) => {
                    error = Some(source.into());
                }
                Ok(Control::Event(Ok(event))) if paths.relevant(&event) => {
                    quiet_at = (Instant::now() + DEBOUNCE).min(limit);
                }
                Ok(Control::Event(Ok(_))) => {}
            }
        }
        error = paths.refresh(&mut watcher).err().or(error);
        on_change(reload(&mut file, error));
    }
}

fn reload(file: &mut ConfigFile, watch_error: Option<WatchError>) -> ConfigUpdate {
    let error = file.reload().err().map(WatchError::from).or(watch_error);
    ConfigUpdate {
        loaded: file.current().clone(),
        error,
    }
}
