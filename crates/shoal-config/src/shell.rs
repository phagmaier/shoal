//! Installation-relative shell assets shared by the daemon and `shell-init`.
use std::{
    io,
    path::{Path, PathBuf},
};

/// Shell scripts and the matching CLI from the running binary's installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellPaths {
    pub root: PathBuf,
    pub binary: PathBuf,
}

impl ShellPaths {
    pub fn discover() -> io::Result<Self> {
        Self::for_executable(&std::env::current_exe()?)
    }

    /// Cargo test executables live one level below the development binaries.
    pub fn for_executable(executable: &Path) -> io::Result<Self> {
        let mut bin = executable
            .parent()
            .ok_or_else(|| io::Error::other("executable has no parent"))?;
        if bin.file_name().is_some_and(|name| name == "deps") {
            bin = bin
                .parent()
                .ok_or_else(|| io::Error::other("test executable has no binary directory"))?;
        }
        let installed = bin.join("../share/shoal/shell");
        let root = if installed.is_dir() {
            installed
        } else if cfg!(debug_assertions) {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../share/shoal/shell")
        } else {
            installed
        };
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("shell integration not found at {}", root.display()),
            ));
        }
        Ok(Self {
            root: root.canonicalize()?,
            binary: bin.join("shoal"),
        })
    }
}
