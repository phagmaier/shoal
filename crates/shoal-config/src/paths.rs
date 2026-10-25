use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("HOME must be an absolute path")]
    MissingHome,
    #[error("XDG_RUNTIME_DIR must be an absolute path")]
    MissingRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdgPaths {
    home: PathBuf,
    config_home: PathBuf,
    state_home: PathBuf,
    cache_home: PathBuf,
    runtime_home: Option<PathBuf>,
}

impl XdgPaths {
    /// Capture process paths once without mutating the environment or filesystem.
    pub fn from_env() -> Result<Self, PathError> {
        Self::from_lookup(|key| std::env::var_os(key))
    }

    /// Injectable lookup for isolated callers and tests. Empty/relative XDG roots
    /// are ignored according to the XDG Base Directory Specification.
    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<Self, PathError> {
        let home = absolute(lookup("HOME")).ok_or(PathError::MissingHome)?;
        Ok(Self {
            config_home: absolute(lookup("XDG_CONFIG_HOME"))
                .unwrap_or_else(|| home.join(".config")),
            state_home: absolute(lookup("XDG_STATE_HOME"))
                .unwrap_or_else(|| home.join(".local/state")),
            cache_home: absolute(lookup("XDG_CACHE_HOME")).unwrap_or_else(|| home.join(".cache")),
            runtime_home: absolute(lookup("XDG_RUNTIME_DIR")),
            home,
        })
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn config_home(&self) -> &Path {
        &self.config_home
    }
    pub fn state_home(&self) -> &Path {
        &self.state_home
    }
    pub fn cache_home(&self) -> &Path {
        &self.cache_home
    }
    pub fn config_dir(&self) -> PathBuf {
        self.config_home.join("shoal")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.state_home.join("shoal")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_home.join("shoal")
    }
    pub fn runtime_dir(&self) -> Result<PathBuf, PathError> {
        self.runtime_home
            .as_ref()
            .map(|p| p.join("shoal"))
            .ok_or(PathError::MissingRuntime)
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }
    pub fn workspace_file(&self) -> PathBuf {
        self.state_dir().join("workspace.json")
    }
    pub fn panes_dir(&self) -> PathBuf {
        self.state_dir().join("panes")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir().join("logs")
    }
    pub fn runtimes_file(&self) -> PathBuf {
        self.cache_dir().join("runtimes.json")
    }
    pub fn socket_file(&self) -> Result<PathBuf, PathError> {
        Ok(self.runtime_dir()?.join("shoald.sock"))
    }
    pub fn lock_file(&self) -> Result<PathBuf, PathError> {
        Ok(self.runtime_dir()?.join("shoald.lock"))
    }
}

fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}
