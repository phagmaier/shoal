//! Owner-only files written atomically (M1 spec §6.1, §6.3). Everything here
//! blocks; the daemon calls it from blocking workers only.

use std::{
    collections::BTreeSet,
    fs::{self, DirBuilder, File, OpenOptions, Permissions},
    io::{self, Write as _},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use shoal_config::XdgPaths;
use shoal_model::PaneId;

use super::{Document, scrollback_path};

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const SCROLLBACK: &str = "scrollback.vt.zst";

/// The state folder and what Shoal keeps in it.
#[derive(Debug, Clone)]
pub struct Store {
    state: PathBuf,
    workspace: PathBuf,
    panes: PathBuf,
}

impl Store {
    pub fn new(paths: &XdgPaths) -> Self {
        Self {
            state: paths.state_dir(),
            workspace: paths.workspace_file(),
            panes: paths.panes_dir(),
        }
    }

    /// Write `workspace.json`, then delete the folders of panes not in `keep`.
    /// Without `scrollback`, kept panes lose their scrollback files too.
    pub fn write_layout(
        &self,
        document: &Document,
        keep: &BTreeSet<PaneId>,
        scrollback: bool,
    ) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(document).map_err(io::Error::other)?;
        private_dir(&self.state)?;
        write_atomic(&self.workspace, |file| file.write_all(&json))?;
        let entries = match fs::read_dir(&self.panes) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            // Leave anything Shoal did not name alone.
            let Some(pane) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<PaneId>().ok())
            else {
                continue;
            };
            if !keep.contains(&pane) {
                fs::remove_dir_all(entry.path())?;
            } else if !scrollback {
                remove_file(&entry.path().join(SCROLLBACK))?;
                remove_file(&entry.path().join(format!("{SCROLLBACK}.tmp")))?;
            }
        }
        Ok(())
    }

    /// Replace a pane's scrollback file with these compressed bytes.
    pub fn write_scrollback(&self, pane: PaneId, compressed: &[u8]) -> io::Result<()> {
        private_dir(&self.state)?;
        private_dir(&self.panes)?;
        let path = self.state.join(scrollback_path(pane));
        private_dir(
            path.parent()
                .expect("invariant: scrollback is inside a pane folder"),
        )?;
        write_atomic(&path, |file| file.write_all(compressed))
    }
}

/// Write `path` through `path.tmp`: fill and fsync the temporary file, rename
/// it over the old one, then fsync the folder. A crash at any point leaves
/// either the old file or the new one, never a mixture.
pub fn write_atomic(path: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .open(&temporary)?;
    // A temporary file left by an earlier crash keeps its old mode.
    file.set_permissions(Permissions::from_mode(FILE_MODE))?;
    fill(&mut file)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
}

/// Create a 0700 folder, or tighten an existing one.
fn private_dir(path: &Path) -> io::Result<()> {
    match DirBuilder::new()
        .mode(DIR_MODE)
        .recursive(true)
        .create(path)
    {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    if fs::metadata(path)?.permissions().mode() & 0o777 != DIR_MODE {
        fs::set_permissions(path, Permissions::from_mode(DIR_MODE))?;
    }
    Ok(())
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}
