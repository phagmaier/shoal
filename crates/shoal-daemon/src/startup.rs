//! Starting and staying single (M1 spec §4.1, steps 1–3).

use std::{
    fs::{self, DirBuilder, File, OpenOptions, Permissions, TryLockError},
    io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};

use shoal_config::{PathError, XdgPaths};

/// The single daemon's hold on the runtime folder.
pub struct Bound {
    /// Held for the daemon's lifetime; closing it releases the lock.
    pub lock: File,
    pub listener: UnixListener,
    pub socket: PathBuf,
    /// Peers must have this uid.
    pub uid: u32,
}

pub enum Start {
    Bound(Bound),
    AlreadyRunning,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Paths(#[from] PathError),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} belongs to another user", path.display())]
    NotOwned { path: PathBuf },
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StartError + '_ {
    move |source| StartError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Create the runtime folder, take the lock and bind the socket. Another
/// daemon holding the lock is not an error.
pub fn start(paths: &XdgPaths) -> Result<Start, StartError> {
    let uid = fs::metadata("/proc/self")
        .map_err(io_error(Path::new("/proc/self")))?
        .uid();
    let dir = paths.runtime_dir()?;
    runtime_dir(&dir, uid)?;

    let lock_path = paths.lock_file()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .map_err(io_error(&lock_path))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Start::AlreadyRunning),
        Err(TryLockError::Error(source)) => return Err(io_error(&lock_path)(source)),
    }

    // Holding the lock means any socket file left here is a dead daemon's.
    let socket = paths.socket_file()?;
    match fs::remove_file(&socket) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(io_error(&socket)(error));
        }
        _ => {}
    }
    let listener = UnixListener::bind(&socket).map_err(io_error(&socket))?;
    fs::set_permissions(&socket, Permissions::from_mode(0o600)).map_err(io_error(&socket))?;
    Ok(Start::Bound(Bound {
        lock,
        listener,
        socket,
        uid,
    }))
}

/// `$XDG_RUNTIME_DIR/shoal/` with mode 0700, owned by `uid`.
fn runtime_dir(dir: &Path, uid: u32) -> Result<(), StartError> {
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io_error(dir)(error)),
    }
    let metadata = fs::symlink_metadata(dir).map_err(io_error(dir))?;
    if !metadata.is_dir() || metadata.uid() != uid {
        return Err(StartError::NotOwned { path: dir.into() });
    }
    if metadata.mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, Permissions::from_mode(0o700)).map_err(io_error(dir))?;
    }
    Ok(())
}
