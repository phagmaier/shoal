//! What a pane's shell is started with (M1 spec §4.2): the program, its
//! environment and its working directory. Pure apart from [`passwd_shell`].

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use shoal_model::PaneId;
use shoal_proto::WINDOW_ENV;

const FALLBACK_SHELL: &str = "/bin/bash";

/// The pane's `argv`, else `shell.program`, else `$SHELL`, else the passwd
/// entry's shell, else `/bin/bash`. A shell runs with no arguments, which on
/// a terminal makes it interactive.
pub fn program(
    argv: &[String],
    shell_program: Option<&str>,
    base_env: &[(OsString, OsString)],
    passwd: impl FnOnce() -> Option<String>,
) -> Vec<OsString> {
    if !argv.is_empty() {
        return argv.iter().map(OsString::from).collect();
    }
    let shell = shell_program
        .map(OsString::from)
        .or_else(|| lookup(base_env, "SHELL").filter(|shell| !shell.is_empty()))
        .or_else(|| passwd().map(OsString::from))
        .unwrap_or_else(|| FALLBACK_SHELL.into());
    vec![shell]
}

/// The daemon's environment, then the window's allowlisted values, then
/// Shoal's own variables. Window values outside [`WINDOW_ENV`] are ignored.
pub fn environment(
    base_env: &[(OsString, OsString)],
    window_env: &BTreeMap<String, String>,
    pane: PaneId,
    socket: &Path,
) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<_, _> = base_env.iter().cloned().collect();
    for (key, value) in window_env {
        if WINDOW_ENV.contains(&key.as_str()) {
            env.insert(key.into(), value.into());
        }
    }
    let shoal = [
        ("TERM", OsStr::new("xterm-256color")),
        ("COLORTERM", OsStr::new("truecolor")),
        ("TERM_PROGRAM", OsStr::new("shoal")),
        (
            "TERM_PROGRAM_VERSION",
            OsStr::new(env!("CARGO_PKG_VERSION")),
        ),
        ("SHOAL_SOCKET", socket.as_os_str()),
    ];
    for (key, value) in shoal {
        env.insert(key.into(), value.into());
    }
    env.insert("SHOAL_PANE".into(), pane.to_string().into());
    env
}

/// Auto-inject only a bare interactive bash/zsh invocation. Explicit command
/// arguments (`-c`, `--norc`, scripts, etc.) retain their requested startup.
pub fn integrate(
    argv: &mut Vec<OsString>,
    env: &mut BTreeMap<OsString, OsString>,
    settings: &super::PaneSettings,
) -> std::io::Result<()> {
    use shoal_config::schema::{PromptMode, ShellIntegration};
    let integration = match settings.shell_integration {
        ShellIntegration::Auto => "auto",
        ShellIntegration::Manual => "manual",
        ShellIntegration::Off => "off",
    };
    let mode = match settings.prompt_mode {
        PromptMode::Native => "native",
        PromptMode::Shell => "shell",
    };
    env.insert("SHOAL_SHELL_INTEGRATION".into(), integration.into());
    env.insert("SHOAL_PROMPT_MODE".into(), mode.into());
    if settings.shell_integration != ShellIntegration::Auto || argv.len() != 1 {
        return Ok(());
    }
    let shell = Path::new(&argv[0]).file_name().and_then(OsStr::to_str);
    if !matches!(shell, Some("bash" | "zsh")) {
        return Ok(());
    }
    let paths = shoal_config::shell::ShellPaths::discover()?;
    env.insert("SHOAL_SHELL_DIR".into(), paths.root.as_os_str().to_owned());
    env.insert("SHOAL_BIN".into(), paths.binary.into_os_string());
    if shell == Some("bash") {
        argv.push("--posix".into());
        env.insert("ENV".into(), paths.root.join("bash/inject.bash").into());
    } else {
        env.remove(OsStr::new("SHOAL_ORIG_ZDOTDIR"));
        if let Some(original) = env.insert("ZDOTDIR".into(), paths.root.join("zsh").into()) {
            env.insert("SHOAL_ORIG_ZDOTDIR".into(), original);
        }
    }
    Ok(())
}

/// The requested directory when it is one, else `$HOME`. Splits and new tabs
/// pass their source pane's directory as the request.
pub fn directory(requested: Option<&Path>, home: &Path) -> PathBuf {
    requested
        .filter(|dir| dir.is_dir())
        .unwrap_or(home)
        .to_owned()
}

/// Our login shell from `/etc/passwd`. Users known only through NSS normally
/// have `$SHELL` set by their login, which is consulted first.
pub fn passwd_shell() -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    shell_for_uid(&passwd, rustix::process::getuid().as_raw())
}

fn shell_for_uid(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let fields: Vec<_> = line.split(':').collect();
        let [_, _, entry_uid, _, _, _, shell] = fields[..] else {
            return None;
        };
        (entry_uid.parse() == Ok(uid) && !shell.is_empty()).then(|| shell.to_owned())
    })
}

fn lookup(env: &[(OsString, OsString)], key: &str) -> Option<OsString> {
    env.iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect()
    }

    fn strings(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn the_program_follows_the_lookup_order() {
        let shell = env(&[("SHELL", "/usr/bin/fish")]);
        let passwd = || Some("/usr/bin/zsh".to_owned());
        let argv = strings(&["nvim", "notes.md"]);
        assert_eq!(
            program(&argv, Some("/bin/zsh"), &shell, passwd),
            ["nvim", "notes.md"]
        );
        assert_eq!(program(&[], Some("/bin/zsh"), &shell, passwd), ["/bin/zsh"]);
        assert_eq!(program(&[], None, &shell, passwd), ["/usr/bin/fish"]);
        assert_eq!(
            program(&[], None, &env(&[("SHELL", "")]), passwd),
            ["/usr/bin/zsh"]
        );
        assert_eq!(program(&[], None, &[], passwd), ["/usr/bin/zsh"]);
        assert_eq!(program(&[], None, &[], || None), ["/bin/bash"]);
    }

    #[test]
    fn the_passwd_shell_is_the_entry_for_our_uid() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      broken line\n\
                      parker:x:1000:1000:Parker:/home/parker:/usr/bin/zsh\n\
                      nologin:x:1001:1001::/:\n";
        assert_eq!(shell_for_uid(passwd, 1000).as_deref(), Some("/usr/bin/zsh"));
        assert_eq!(shell_for_uid(passwd, 0).as_deref(), Some("/bin/bash"));
        assert_eq!(shell_for_uid(passwd, 1001), None);
        assert_eq!(shell_for_uid(passwd, 4242), None);
    }

    #[test]
    fn the_environment_layers_window_values_and_shoals_own() {
        let base = env(&[
            ("PATH", "/usr/bin"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("TERM", "linux"),
            ("SHOAL_PANE", "p1"),
        ]);
        let window = BTreeMap::from([
            ("WAYLAND_DISPLAY".to_owned(), "wayland-1".to_owned()),
            ("SSH_AUTH_SOCK".to_owned(), "/run/user/1000/ssh".to_owned()),
            ("PATH".to_owned(), "/evil".to_owned()),
        ]);
        let got = environment(&base, &window, PaneId::new(40), Path::new("/run/s.sock"));
        let expected = env(&[
            ("COLORTERM", "truecolor"),
            ("PATH", "/usr/bin"),
            ("SHOAL_PANE", "p40"),
            ("SHOAL_SOCKET", "/run/s.sock"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
            ("TERM", "xterm-256color"),
            ("TERM_PROGRAM", "shoal"),
            ("TERM_PROGRAM_VERSION", "0.1.0"),
            ("WAYLAND_DISPLAY", "wayland-1"),
        ]);
        assert_eq!(got.into_iter().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn a_missing_directory_falls_back_to_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new("/home/parker");
        assert_eq!(directory(Some(dir.path()), home), dir.path());
        let gone = dir.path().join("gone");
        assert_eq!(directory(Some(&gone), home), home);
        assert_eq!(directory(None, home), home);
    }
}
