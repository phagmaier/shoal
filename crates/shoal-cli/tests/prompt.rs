//! `shoal prompt` end to end (M1 spec §8.3): the real binary in temporary git
//! repositories under a temporary `HOME`, rendered by real bash and zsh.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant, SystemTime},
};

const SEP: char = '\u{e0b0}';
const BRANCH: char = '\u{e0a0}';
const NODE: char = '\u{e718}';
/// M1 spec §8.3's hard cap.
const CAP: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
enum Shell {
    Bash,
    Zsh,
}

const SHELLS: [Shell; 2] = [Shell::Bash, Shell::Zsh];

impl Shell {
    fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
        }
    }

    /// What the shell shows for `prompt`, with colors and non-printing markers
    /// removed. Bash renders it as with its default `promptvars`; zsh as with
    /// `prompt_subst`, which expands `$` and backquotes the way bash does.
    fn show(self, prompt: &str, cwd: &Path) -> String {
        let script = match self {
            Self::Bash => r#"PS1=$1; printf %s "${PS1@P}""#,
            Self::Zsh => r#"print -rn -- "${(%)${(e)1}}""#,
        };
        self.run(script, prompt, cwd)
    }

    /// The same, as with bash's `promptvars` off or zsh's `prompt_subst` off.
    fn show_unexpanded(self, prompt: &str, cwd: &Path) -> String {
        let script = match self {
            Self::Bash => r#"shopt -u promptvars; PS1=$1; printf %s "${PS1@P}""#,
            Self::Zsh => r#"print -rn -- "${(%)1}""#,
        };
        self.run(script, prompt, cwd)
    }

    /// Run `script` with `prompt` as `$1`, reading no startup files.
    fn run(self, script: &str, prompt: &str, cwd: &Path) -> String {
        let no_rc = match self {
            Self::Bash => "--norc",
            Self::Zsh => "-f",
        };
        let output = Command::new(self.name())
            .args([no_rc, "-c", script, "_", prompt])
            .current_dir(cwd)
            .output()
            .expect("run the shell");
        assert!(output.status.success(), "{self:?}: {output:?}");
        strip_escapes(&String::from_utf8(output.stdout).expect("UTF-8 prompt"))
    }
}

/// Remove SGR sequences and readline's \001 \002 markers.
fn strip_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        match character {
            '\x1b' => {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            }
            '\x01' | '\x02' => {}
            character => out.push(character),
        }
    }
    out
}

/// A temporary `HOME` with no config and no Omarchy theme.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp home"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.path().join(name);
        fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(path, contents).expect("write file");
    }

    /// An executable shell script in `bin/`, which `run_with_bin` puts first on `PATH`.
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    /// Run git with a fixed identity, dates and no user or system config.
    fn git(&self, dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", self.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Shoal")
            .env("GIT_AUTHOR_EMAIL", "shoal@example.com")
            .env("GIT_AUTHOR_DATE", "2026-10-25T12:00:00Z")
            .env("GIT_COMMITTER_NAME", "Shoal")
            .env("GIT_COMMITTER_EMAIL", "shoal@example.com")
            .env("GIT_COMMITTER_DATE", "2026-10-25T12:00:00Z")
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    fn commit(&self, dir: &Path, file: &str, contents: &str) {
        fs::write(dir.join(file), contents).expect("write file");
        self.git(dir, &["add", file]);
        self.git(dir, &["commit", "-q", "-m", file]);
    }

    /// A repository on `main` with one commit.
    fn repo(&self, name: &str) -> PathBuf {
        let dir = self.dir(name);
        self.git(&dir, &["init", "-q", "-b", "main"]);
        self.commit(&dir, "notes.txt", "one\n");
        dir
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args, &[]).output().expect("run shoal")
    }

    fn command(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
        let path = std::env::var("PATH").unwrap_or_default();
        let mut command = Command::new(env!("CARGO_BIN_EXE_shoal"));
        command
            .arg("prompt")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("HOME", self.path())
            .env("USER", "parker")
            .env("PWD", cwd)
            .env(
                "PATH",
                format!("{}:{path}", self.path().join("bin").display()),
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env.iter().copied());
        command
    }

    /// The raw prompt for `shell`, as `$(...)` would capture it.
    fn prompt(&self, shell: Shell, cwd: &Path) -> String {
        self.prompt_with(shell, cwd, &[])
    }

    fn prompt_with(&self, shell: Shell, cwd: &Path, args: &[&str]) -> String {
        let args = [&["--shell", shell.name()], args].concat();
        let output = self.run(cwd, &args);
        assert!(output.status.success(), "shoal prompt: {output:?}");
        let text = String::from_utf8(output.stdout).expect("UTF-8 prompt");
        text.strip_suffix('\n').expect("one line").to_owned()
    }
}

/// The raw prompt with escape bytes visible, for golden files.
fn golden(prompt: &str) -> String {
    prompt.replace('\x1b', "\\e")
}

/// For each shell: the visible prompt matches `expected`, and the raw bytes
/// match the golden file `<name>_<shell>`.
fn check_state(home: &Home, cwd: &Path, name: &str, expected: &str) {
    for shell in SHELLS {
        let prompt = home.prompt(shell, cwd);
        assert_eq!(
            shell.show(&prompt, home.path()),
            expected,
            "{name} in {shell:?}"
        );
        insta::assert_snapshot!(format!("{name}_{}", shell.name()), golden(&prompt));
    }
}

#[test]
fn clean_repository() {
    let home = Home::new();
    let repo = home.repo("clean");
    check_state(
        &home,
        &repo,
        "clean",
        &format!(" parker {SEP} ~/clean {SEP} {BRANCH} main {SEP} ❯ "),
    );
}

#[test]
fn dirty_repository() {
    let home = Home::new();
    let repo = home.repo("dirty");
    fs::write(repo.join("notes.txt"), "two\n").expect("edit");
    check_state(
        &home,
        &repo,
        "dirty",
        &format!(" parker {SEP} ~/dirty {SEP} {BRANCH} main ● {SEP} ❯ "),
    );
}

#[test]
fn ahead_and_behind_a_local_bare_remote() {
    let home = Home::new();
    let remote = home.dir("remote.git");
    home.git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let repo = home.repo("work");
    home.git(&repo, &["remote", "add", "origin", "../remote.git"]);
    home.git(&repo, &["push", "-q", "-u", "origin", "main"]);
    home.git(home.path(), &["clone", "-q", "remote.git", "other"]);
    home.commit(&home.path().join("other"), "theirs.txt", "theirs\n");
    home.git(&home.path().join("other"), &["push", "-q"]);
    home.commit(&repo, "mine.txt", "mine\n");
    home.commit(&repo, "mine2.txt", "mine too\n");
    home.git(&repo, &["fetch", "-q"]);
    check_state(
        &home,
        &repo,
        "ahead_behind",
        &format!(" parker {SEP} ~/work {SEP} {BRANCH} main ⇡2 ⇣1 {SEP} ❯ "),
    );
}

#[test]
fn detached_head_shows_a_short_hash() {
    let home = Home::new();
    let repo = home.repo("detached");
    home.git(&repo, &["checkout", "-q", "--detach"]);
    let output = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(&repo)
        .output()
        .expect("rev-parse");
    let hash = String::from_utf8(output.stdout).expect("hash");
    check_state(
        &home,
        &repo,
        "detached",
        &format!(
            " parker {SEP} ~/detached {SEP} {BRANCH} {} {SEP} ❯ ",
            hash.trim()
        ),
    );
}

#[test]
fn outside_a_repository_there_is_no_git_segment() {
    let home = Home::new();
    let dir = home.dir("plain/deep/er/still");
    check_state(
        &home,
        &dir,
        "none",
        &format!(" parker {SEP} …/deep/er/still {SEP} ❯ "),
    );
}

#[test]
fn a_slow_git_shows_the_branch_and_a_question_mark_within_the_cap() {
    let home = Home::new();
    let repo = home.repo("slow");
    home.script("git", "exec sleep 5");
    for shell in SHELLS {
        let started = Instant::now();
        let prompt = home.prompt(shell, &repo);
        let elapsed = started.elapsed();
        assert_eq!(
            shell.show(&prompt, home.path()),
            format!(" parker {SEP} ~/slow {SEP} {BRANCH} main ? {SEP} ❯ ")
        );
        assert!(elapsed < CAP, "{shell:?} took {elapsed:?}");
    }
}

#[test]
fn names_that_look_like_shell_code_are_shown_not_run() {
    let home = Home::new();
    let dir = home.dir("$(touch pwned) `touch pwned` 100%");
    home.git(
        &dir,
        &["init", "-q", "-b", "$(touch${IFS}pwned)`touch${IFS}pwned`%"],
    );
    let shown = format!(
        " parker {SEP} ~/$(touch pwned) `touch pwned` 100% {SEP} \
         {BRANCH} $(touch${{IFS}}pwned)`touch${{IFS}}pwned`% ● {SEP} ❯ "
    );
    fs::write(dir.join("untracked"), "").expect("untracked file");
    for shell in SHELLS {
        let prompt = home.prompt(shell, &dir);
        assert_eq!(shell.show(&prompt, &dir), shown, "{shell:?}");
        let unexpanded = shell.show_unexpanded(&prompt, &dir);
        assert!(
            unexpanded.contains("touch pwned"),
            "{shell:?}: {unexpanded}"
        );
    }
    assert!(!dir.join("pwned").exists(), "a prompt ran a command");
    assert!(
        !home.path().join("pwned").exists(),
        "a prompt ran a command"
    );
}

#[test]
fn ssh_sessions_add_the_host() {
    let home = Home::new();
    let dir = home.dir("code");
    let host = fs::read_to_string("/proc/sys/kernel/hostname").expect("hostname");
    let output = home
        .command(
            &dir,
            &["--shell", "bash"],
            &[("SSH_CONNECTION", "10.0.0.2 51000 10.0.0.1 22")],
        )
        .output()
        .expect("run shoal");
    let prompt = String::from_utf8(output.stdout).expect("UTF-8");
    assert_eq!(
        Shell::Bash.show(prompt.trim_end_matches('\n'), home.path()),
        format!(" parker@{} {SEP} ~/code {SEP} ❯ ", host.trim_end())
    );
}

#[test]
fn a_failure_turns_the_character_red_and_shows_the_time_and_jobs() {
    let home = Home::new();
    let dir = home.dir("code");
    let args = [
        "--status",
        "1",
        "--duration-ms",
        "12400",
        "--jobs",
        "2",
        "--cols",
        "80",
    ];
    let prompt = home.prompt_with(Shell::Zsh, &dir, &args);
    assert_eq!(
        Shell::Zsh.show(&prompt, home.path()),
        format!(" parker {SEP} ~/code {SEP} took 12s ✦2 ❯ ")
    );
    // The built-in palette's red, #f7768e.
    assert!(
        prompt.contains("\x1b[38;2;247;118;142m%}❯"),
        "{}",
        golden(&prompt)
    );
}

#[test]
fn config_and_omarchy_theme_set_the_style_segments_and_colors() {
    let home = Home::new();
    home.write(
        ".local/state/omarchy/current/theme/colors.toml",
        "blue = \"#0000ff\"\naccent = \"#00ff00\"\n",
    );
    home.write(
        ".config/shoal/config.toml",
        "[prompt]\nstyle = \"lean\"\nsegments = [\"path\", \"git\"]\n\
         [theme.colors]\naccent = \"#ff0000\"\n",
    );
    let repo = home.repo("code");
    let prompt = home.prompt(Shell::Bash, &repo);
    assert_eq!(
        Shell::Bash.show(&prompt, home.path()),
        format!("~/code {BRANCH} main ❯ ")
    );
    assert!(
        prompt.starts_with("\\[\x1b[38;2;0;0;255m\\]~/code"),
        "{}",
        golden(&prompt)
    );
    assert!(
        prompt.contains("\x1b[38;2;255;0;0m\\]❯"),
        "{}",
        golden(&prompt)
    );
}

#[test]
fn runtime_versions_are_cached_by_program_path_and_mtime() {
    let home = Home::new();
    let dir = home.dir("app");
    fs::write(dir.join("package.json"), "{}").expect("marker");
    let runs = home.path().join("node-runs");
    let node = home.script(
        "node",
        &format!("echo run >> '{}'\necho v22.11.0", runs.display()),
    );
    let expected = format!(" parker {SEP} ~/app {SEP} {NODE} 22.11 {SEP} ❯ ");
    let count = || {
        fs::read_to_string(&runs)
            .unwrap_or_default()
            .lines()
            .count()
    };

    for _ in 0..2 {
        let prompt = home.prompt(Shell::Bash, &dir);
        assert_eq!(Shell::Bash.show(&prompt, home.path()), expected);
    }
    assert_eq!(count(), 1, "the second prompt used the cache");
    assert!(home.path().join(".cache/shoal/runtimes.json").is_file());

    let earlier = SystemTime::now() - Duration::from_secs(3600);
    fs::File::open(&node)
        .and_then(|file| file.set_modified(earlier))
        .expect("set mtime");
    let prompt = home.prompt(Shell::Bash, &dir);
    assert_eq!(Shell::Bash.show(&prompt, home.path()), expected);
    assert_eq!(count(), 2, "a changed mtime asks the program again");
}

#[test]
fn version_manager_shims_are_asked_every_time() {
    let home = Home::new();
    let dir = home.dir("app");
    fs::write(dir.join(".nvmrc"), "22\n").expect("marker");
    let runs = home.path().join("mise-runs");
    // mise's shims are links to the mise binary, which picks the version.
    let mise = home.path().join("tools/mise");
    fs::create_dir_all(mise.parent().expect("parent")).expect("tools dir");
    fs::write(
        &mise,
        format!(
            "#!/bin/sh\necho run >> '{}'\necho v22.11.0\n",
            runs.display()
        ),
    )
    .expect("fake mise");
    fs::set_permissions(&mise, fs::Permissions::from_mode(0o755)).expect("chmod");
    std::os::unix::fs::symlink(&mise, home.dir("bin").join("node")).expect("shim");

    for _ in 0..2 {
        let prompt = home.prompt(Shell::Bash, &dir);
        assert_eq!(
            Shell::Bash.show(&prompt, home.path()),
            format!(" parker {SEP} ~/app {SEP} {NODE} 22.11 {SEP} ❯ ")
        );
    }
    let count = fs::read_to_string(&runs).expect("runs").lines().count();
    assert_eq!(count, 2, "each prompt asked the shim");
    assert!(!home.path().join(".cache/shoal/runtimes.json").exists());
}

#[test]
fn a_slow_runtime_shows_a_question_mark_within_the_cap() {
    let home = Home::new();
    let dir = home.dir("app");
    fs::write(dir.join("Cargo.toml"), "").expect("marker");
    home.script("rustc", "exec sleep 5");
    let started = Instant::now();
    let prompt = home.prompt(Shell::Zsh, &dir);
    let elapsed = started.elapsed();
    assert_eq!(
        Shell::Zsh.show(&prompt, home.path()),
        format!(" parker {SEP} ~/app {SEP} \u{e7a8} ? {SEP} ❯ ")
    );
    assert!(elapsed < CAP, "took {elapsed:?}");
}

#[test]
fn bad_usage_exits_with_two() {
    let home = Home::new();
    let output = home.run(home.path(), &["--shell", "fish"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).lines().next(),
        Some("shoal prompt: --shell must be bash or zsh")
    );
}
