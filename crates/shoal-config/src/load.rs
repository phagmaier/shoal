use crate::Config;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoadedConfig {
    pub config: Config,
    pub warnings: Vec<ConfigWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: unknown config key {key}", path.display())]
pub struct ConfigWarning {
    pub path: PathBuf,
    pub key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{}:{line}: {reason}", path.display())]
    Invalid {
        path: PathBuf,
        line: usize,
        reason: String,
    },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Config {
    /// Parse an entire edit atomically, retaining source locations on errors.
    pub fn parse(path: &Path, source: &str) -> Result<LoadedConfig, ConfigError> {
        let invalid = |error: toml::de::Error| ConfigError::Invalid {
            path: path.into(),
            line: line_at(source.as_bytes(), error.span().map_or(0, |span| span.start)),
            reason: error.message().to_owned(),
        };
        let de = toml::de::Deserializer::parse(source).map_err(invalid)?;
        let mut warnings = Vec::new();
        let config = serde_ignored::deserialize(de, |key| {
            warnings.push(ConfigWarning {
                path: path.into(),
                key: key.to_string(),
            })
        })
        .map_err(invalid)?;
        Ok(LoadedConfig { config, warnings })
    }

    /// A missing file is the default config; other I/O failures are reported.
    pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadedConfig::default());
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.into(),
                    source,
                });
            }
        };
        let source = std::str::from_utf8(&bytes).map_err(|error| ConfigError::Invalid {
            path: path.into(),
            line: line_at(&bytes, error.valid_up_to()),
            reason: "config is not valid UTF-8".into(),
        })?;
        Self::parse(path, source)
    }
}

/// Synchronous reload state, usable by a watcher or a caller doing its own I/O.
/// New instances start with defaults, including when the first edit is invalid.
#[derive(Debug)]
pub struct ConfigFile {
    path: PathBuf,
    current: LoadedConfig,
}

impl ConfigFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            current: LoadedConfig::default(),
        }
    }
    pub fn current(&self) -> &LoadedConfig {
        &self.current
    }
    pub fn reload(&mut self) -> Result<&LoadedConfig, ConfigError> {
        let loaded = Config::load(&self.path)?;
        self.current = loaded;
        Ok(&self.current)
    }
}

pub(crate) fn line_at(bytes: &[u8], offset: usize) -> usize {
    1 + bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
}
