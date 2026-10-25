use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use serde::{Deserialize, Serialize};

use super::command::Running;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Node,
    Python,
    Rust,
    Go,
    Ruby,
}

/// Marker files in the order they are checked; the first present one wins.
const MARKERS: [(&str, Language); 7] = [
    ("package.json", Language::Node),
    (".nvmrc", Language::Node),
    ("pyproject.toml", Language::Python),
    (".python-version", Language::Python),
    ("Cargo.toml", Language::Rust),
    ("go.mod", Language::Go),
    ("Gemfile", Language::Ruby),
];

impl Language {
    /// Nerd Font icons, which the window's bundled symbols font draws.
    pub fn icon(self) -> char {
        match self {
            Self::Node => '\u{e718}',
            Self::Python => '\u{e73c}',
            Self::Rust => '\u{e7a8}',
            Self::Go => '\u{e627}',
            Self::Ruby => '\u{e739}',
        }
    }

    /// Programs to look for on `PATH`, in order.
    fn programs(self) -> &'static [&'static str] {
        match self {
            Self::Node => &["node"],
            Self::Python => &["python", "python3"],
            Self::Rust => &["rustc"],
            Self::Go => &["go"],
            Self::Ruby => &["ruby"],
        }
    }

    fn version_args(self) -> &'static [&'static str] {
        match self {
            // Go has no --version flag.
            Self::Go => &["version"],
            _ => &["--version"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub language: Language,
    /// `major.minor`; `None` when the version is unknown, such as when the
    /// version command was still running at the hard cap.
    pub version: Option<String>,
}

/// A runtime version read from the cache or being asked of the program.
pub struct Probe {
    language: Language,
    lookup: Lookup,
}

enum Lookup {
    Cached(String),
    Asking {
        running: Option<Running>,
        key: String,
        mtime: Mtime,
        cache_file: Option<PathBuf>,
        cache: Cache,
    },
}

/// Start finding the version for the first marker file in `cwd`. `None` when
/// there is no marker file or its program is not on `path`.
pub fn start(
    cwd: &Path,
    path: Option<&std::ffi::OsStr>,
    cache_file: Option<PathBuf>,
) -> Option<Probe> {
    let language = MARKERS
        .iter()
        .find(|(marker, _)| cwd.join(marker).exists())
        .map(|&(_, language)| language)?;
    let (program, metadata) = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .find_map(|dir| {
            // A relative PATH entry would make the result depend on the directory.
            if !dir.is_absolute() {
                return None;
            }
            language.programs().iter().find_map(|name| {
                let program = dir.join(name);
                let metadata = std::fs::metadata(&program).ok()?;
                (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
                    .then_some((program, metadata))
            })
        })?;
    let mtime = Mtime {
        seconds: metadata.mtime(),
        nanoseconds: metadata.mtime_nsec(),
    };
    let key = program.to_string_lossy().into_owned();
    // A shim's path and mtime stay the same while the version it runs follows
    // each project's pin, so its answer is never cached.
    let cache_file = cache_file.filter(|_| !is_shim(&program));
    let cache = cache_file.as_deref().map(Cache::read).unwrap_or_default();
    let lookup = match cache.0.get(&key) {
        Some(entry) if entry.mtime == mtime => Lookup::Cached(entry.version.clone()),
        _ => Lookup::Asking {
            running: Running::spawn(Command::new(&program).args(language.version_args())),
            key,
            mtime,
            cache_file,
            cache,
        },
    };
    Some(Probe { language, lookup })
}

impl Probe {
    /// The version, waiting for the program until `deadline` and caching what it
    /// reports.
    pub fn finish(self, deadline: Instant) -> Runtime {
        let version = match self.lookup {
            Lookup::Cached(version) => Some(version),
            Lookup::Asking {
                running,
                key,
                mtime,
                cache_file,
                mut cache,
            } => {
                let version = running
                    .and_then(|running| running.output_by(deadline))
                    .and_then(|output| parse_version(&String::from_utf8_lossy(&output)));
                if let (Some(version), Some(cache_file)) = (&version, cache_file) {
                    let entry = Entry {
                        mtime,
                        version: version.clone(),
                    };
                    cache.0.insert(key, entry);
                    cache.write(&cache_file);
                }
                version
            }
        };
        Runtime {
            language: self.language,
            version,
        }
    }
}

/// A version manager's stand-in: a link to `mise` or `rustup`, or a script in
/// a `shims` directory as asdf, pyenv, rbenv and nodenv install.
fn is_shim(program: &Path) -> bool {
    let manager = std::fs::canonicalize(program).is_ok_and(|target| {
        target
            .file_name()
            .is_some_and(|name| name == "mise" || name == "rustup")
    });
    manager
        || program
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == "shims")
}

/// `runtimes.json`: versions by program path, valid while the mtime matches.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache(BTreeMap<String, Entry>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    mtime: Mtime,
    version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Mtime {
    seconds: i64,
    nanoseconds: i64,
}

impl Cache {
    /// A missing or unreadable cache is empty; it only saves time.
    fn read(file: &Path) -> Self {
        std::fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Replace the file atomically, so concurrent prompts never read half of
    /// one. Failures are ignored: the next prompt asks the program again.
    fn write(&self, file: &Path) {
        let (Some(dir), Ok(json)) = (file.parent(), serde_json::to_vec(self)) else {
            return;
        };
        let temp = file.with_extension(format!("json.{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&temp, json))
            .and_then(|()| std::fs::rename(&temp, file));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
}

/// The first `major.minor` in a program's version output.
pub fn parse_version(output: &str) -> Option<String> {
    let bytes = output.as_bytes();
    (0..bytes.len()).find_map(|start| {
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            return None;
        }
        let digits = |from: usize| {
            bytes[from..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count()
        };
        let major = digits(start);
        let dot = start + major;
        let minor = (major > 0 && bytes.get(dot) == Some(&b'.')).then(|| digits(dot + 1))?;
        (minor > 0).then(|| output[start..dot + 1 + minor].to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_major_and_minor_from_real_version_output() {
        for (output, version) in [
            ("v22.11.0\n", "22.11"),
            ("Python 3.13.1\n", "3.13"),
            ("rustc 1.98.1 (2b4c1a7e5 2026-09-17)\n", "1.98"),
            ("go version go1.23.4 linux/amd64\n", "1.23"),
            (
                "ruby 3.3.6 (2024-11-05 revision 75015d4c1f) [x86_64-linux]\n",
                "3.3",
            ),
        ] {
            assert_eq!(parse_version(output).as_deref(), Some(version), "{output}");
        }
    }

    #[test]
    fn needs_a_dotted_number() {
        for output in ["", "node\n", "version 22\n", "v22.\n", ".5\n"] {
            assert_eq!(parse_version(output), None, "{output:?}");
        }
    }

    #[test]
    fn recognizes_version_manager_shims() {
        let dir = tempfile::tempdir().expect("temp dir");
        let write = |path: &Path| {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(path, "#!/bin/sh\n").expect("write");
        };
        let rustup = dir.path().join("bin/rustup");
        let pyenv_shim = dir.path().join(".pyenv/shims/python");
        let real = dir.path().join("node/22.11.0/bin/node");
        for path in [&rustup, &pyenv_shim, &real] {
            write(path);
        }
        let proxy = dir.path().join("bin/rustc");
        std::os::unix::fs::symlink("rustup", &proxy).expect("symlink");
        let linked = dir.path().join("bin/node");
        std::os::unix::fs::symlink(&real, &linked).expect("symlink");

        assert!(is_shim(&proxy));
        assert!(is_shim(&pyenv_shim));
        assert!(!is_shim(&real));
        assert!(!is_shim(&linked), "a link to a real program is cached");
    }

    #[test]
    fn a_damaged_cache_reads_as_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("runtimes.json");
        std::fs::write(&file, "{ not json").expect("cache");
        assert!(Cache::read(&file).0.is_empty());
        assert!(Cache::read(&dir.path().join("missing.json")).0.is_empty());
    }
}
