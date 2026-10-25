use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    Config, XdgPaths,
    load::line_at,
    palette::{self, Layer, Palette, ThemeColors},
    schema::{Rgb, ThemeConfig},
};

/// Where theme colors come from, with paths already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSource {
    /// The `omarchy/current/` folders to probe: Omarchy 4's, then 3.x's.
    Omarchy([PathBuf; 2]),
    /// A `colors.toml`-style file named by `theme.source`.
    File(PathBuf),
}

impl ThemeSource {
    pub fn from_config(theme: &ThemeConfig, paths: &XdgPaths) -> Self {
        if theme.source == "omarchy" {
            return Self::Omarchy(
                [paths.state_home(), paths.config_home()].map(|root| root.join("omarchy/current")),
            );
        }
        let path = Path::new(&theme.source);
        Self::File(if let Ok(rest) = path.strip_prefix("~") {
            paths.home().join(rest)
        } else {
            paths.config_dir().join(path)
        })
    }

    /// Read the active theme now. Problems become warnings, never errors.
    pub fn load(&self) -> LoadedTheme {
        let mut theme = LoadedTheme {
            folder: None,
            layers: Vec::new(),
            warnings: Vec::new(),
        };
        let files = match self {
            Self::Omarchy(currents) => {
                theme.folder = currents
                    .iter()
                    .map(|current| current.join("theme"))
                    .find(|folder| folder.is_dir());
                let Some(folder) = &theme.folder else {
                    return theme;
                };
                vec![
                    (folder.join("shoal.toml"), false),
                    (folder.join("colors.toml"), false),
                ]
            }
            Self::File(path) => vec![(path.clone(), true)],
        };
        for (path, required) in files {
            if let Some(layer) = read_layer(&path, required, &mut theme.warnings) {
                theme.layers.push(layer);
            }
        }
        theme
    }

    /// Files whose changes, or whose symlink targets' changes, are theme changes.
    pub(crate) fn watched_files(&self) -> Vec<PathBuf> {
        match self {
            Self::Omarchy(currents) => currents
                .iter()
                .flat_map(|current| {
                    [
                        current.join("theme/colors.toml"),
                        current.join("theme/shoal.toml"),
                        current.join("theme.name"),
                    ]
                })
                .collect(),
            Self::File(path) => vec![path.clone()],
        }
    }
}

/// Theme files as read, before config overrides are layered on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTheme {
    /// The Omarchy theme folder in use; `None` for a file source or when neither
    /// Omarchy location has one, in which case the built-in palette applies.
    pub folder: Option<PathBuf>,
    // shoal.toml then colors.toml, or the single file source.
    layers: Vec<Layer>,
    pub warnings: Vec<ThemeWarning>,
}

impl LoadedTheme {
    /// Resolve every palette key with `[theme.colors]` above the theme files.
    pub fn colors(&self, config: &ThemeConfig) -> ThemeColors {
        let overrides = palette::layer(config.colors.iter().map(|(key, &rgb)| (&**key, rgb)));
        let layers: Vec<_> = std::iter::once(overrides)
            .chain(self.layers.iter().cloned())
            .collect();
        ThemeColors::resolve(&layers)
    }

    pub fn palette(&self, config: &Config) -> Palette {
        self.colors(&config.theme).palette(&config.cursor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{}: {reason}", path.display(), line.map(|line| format!(":{line}")).unwrap_or_default())]
pub struct ThemeWarning {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub reason: String,
}

fn read_layer(path: &Path, required: bool, warnings: &mut Vec<ThemeWarning>) -> Option<Layer> {
    let mut warn = |line, reason| {
        warnings.push(ThemeWarning {
            path: path.into(),
            line,
            reason,
        })
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => return None,
        Err(error) => {
            warn(None, error.to_string());
            return None;
        }
    };
    let source = match std::str::from_utf8(&bytes) {
        Ok(source) => source,
        Err(error) => {
            let line = line_at(&bytes, error.valid_up_to());
            warn(Some(line), "theme file is not valid UTF-8".into());
            return None;
        }
    };
    let table = match toml::from_str::<BTreeMap<String, toml::Spanned<toml::Value>>>(source) {
        Ok(table) => table,
        Err(error) => {
            let line = line_at(source.as_bytes(), error.span().map_or(0, |span| span.start));
            warn(Some(line), error.message().to_owned());
            return None;
        }
    };
    let mut entries = Vec::new();
    for (key, value) in &table {
        if palette::used_key(key).is_none() {
            continue;
        }
        match value.get_ref().as_str().map(str::parse::<Rgb>) {
            Some(Ok(rgb)) => entries.push((key.as_str(), rgb)),
            _ => warn(
                Some(line_at(source.as_bytes(), value.span().start)),
                format!("{key}: color must be #rrggbb"),
            ),
        }
    }
    Some(palette::layer(entries))
}
