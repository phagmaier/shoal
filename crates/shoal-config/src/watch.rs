use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{ConfigFile, LoadedConfig, LoadedTheme, ThemeSource};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("file watcher: {0}")]
    Notify(#[from] notify::Error),
    #[error("file watcher: {0}")]
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

/// A theme reread. Unusable theme files are warnings inside `theme`; `error`
/// only reports a problem with watching itself.
#[derive(Debug)]
pub struct ThemeUpdate {
    pub theme: LoadedTheme,
    pub error: Option<WatchError>,
}

/// Native config watching. Callbacks run on the worker thread and should forward
/// updates promptly. Dropping the watcher stops and joins that worker.
pub struct ConfigWatcher {
    _worker: Worker,
}

/// Native watching of the active theme, including Omarchy's theme-folder swaps,
/// on the same terms as [`ConfigWatcher`]. A changed `theme.source` needs a new
/// watcher; a changed `[theme.colors]` needs only [`LoadedTheme::colors`].
pub struct ThemeWatcher {
    _worker: Worker,
}

struct Worker {
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
        let mut file = ConfigFile::new(std::path::absolute(path)?);
        let (worker, initial) = Worker::spawn(
            "shoal-config",
            vec![path.to_path_buf()],
            move |watch_error| {
                let error = file.reload().err().map(WatchError::from).or(watch_error);
                ConfigUpdate {
                    loaded: file.current().clone(),
                    error,
                }
            },
            on_change,
        )?;
        Ok((Self { _worker: worker }, initial))
    }
}

impl ThemeWatcher {
    /// Install watches before the initial read; nothing gets created.
    pub fn new(
        source: ThemeSource,
        on_change: impl FnMut(ThemeUpdate) + Send + 'static,
    ) -> Result<(Self, ThemeUpdate), WatchError> {
        let (worker, initial) = Worker::spawn(
            "shoal-theme",
            source.watched_files(),
            move |error| ThemeUpdate {
                theme: source.load(),
                error,
            },
            on_change,
        )?;
        Ok((Self { _worker: worker }, initial))
    }
}

impl Worker {
    fn spawn<U>(
        name: &str,
        files: Vec<PathBuf>,
        mut load: impl FnMut(Option<WatchError>) -> U + Send + 'static,
        on_change: impl FnMut(U) + Send + 'static,
    ) -> Result<(Self, U), WatchError> {
        let (control, receiver) = mpsc::channel();
        let events = control.clone();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = events.send(Control::Event(event));
        })?;
        let mut paths = WatchPaths {
            files: files
                .iter()
                .map(std::path::absolute)
                .collect::<Result<_, _>>()?,
            targets: Vec::new(),
            active: BTreeSet::new(),
        };
        paths.refresh(&mut watcher)?;
        let initial = load(None);
        let worker = thread::Builder::new().name(name.into()).spawn(move || {
            run(watcher, paths, receiver, load, on_change);
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

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct WatchPaths {
    files: Vec<PathBuf>,
    targets: Vec<PathBuf>,
    active: BTreeSet<PathBuf>,
}

impl WatchPaths {
    /// An event touches a watched file, its symlink target or an ancestor of
    /// either; siblings such as Omarchy's `next-theme` staging folder do not count.
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
            self.files
                .iter()
                .chain(&self.targets)
                .any(|file| file.starts_with(path))
        })
    }

    fn refresh(&mut self, watcher: &mut RecommendedWatcher) -> Result<(), WatchError> {
        self.targets = self.files.iter().filter_map(|file| target(file)).collect();
        // Nonrecursive ancestor watches catch future directories and replacements
        // without recursively watching the user's home. Rearm even identical
        // paths: an atomic directory replacement may have invalidated the inode.
        let desired: BTreeSet<_> = self
            .files
            .iter()
            .chain(&self.targets)
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

/// Where a watched file really lives. inotify follows symlinked directories, and
/// a directory watched under two names reports events under only one of them,
/// so even a missing file needs its canonical spelling.
fn target(file: &Path) -> Option<PathBuf> {
    let parent = file.parent().unwrap_or(Path::new("/"));
    if let Ok(path) = file.canonicalize() {
        Some(path)
    } else if let Ok(link) = std::fs::read_link(file) {
        Some(if link.is_absolute() {
            link
        } else {
            parent.join(link)
        })
    } else {
        Some(parent.canonicalize().ok()?.join(file.file_name()?))
    }
}

fn run<U>(
    mut watcher: RecommendedWatcher,
    mut paths: WatchPaths,
    receiver: Receiver<Control>,
    mut load: impl FnMut(Option<WatchError>) -> U,
    mut on_change: impl FnMut(U),
) {
    loop {
        // A watch error may mean missed events, so it rereads like a change.
        let mut error = match receiver.recv() {
            Ok(Control::Stop) | Err(_) => return,
            Ok(Control::Event(Err(source))) => Some(source.into()),
            Ok(Control::Event(Ok(event))) if paths.relevant(&event) => None,
            Ok(Control::Event(Ok(_))) => continue,
        };
        let limit = Instant::now() + MAX_BURST;
        let mut quiet_at = Instant::now() + DEBOUNCE;
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
        on_change(load(error));
    }
}
