use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use super::command::Running;

/// How long `git status` may run before the prompt falls back to `.git/HEAD`.
pub const TIMEOUT: Duration = Duration::from_millis(80);
/// Hex digits shown for a detached head.
const SHORT_HASH: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// A short commit hash.
    Detached(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub head: Head,
    pub dirty: bool,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Git {
    Known(Status),
    /// `git status` timed out or failed; the head comes from `.git/HEAD` when
    /// that is readable.
    Unknown(Option<Head>),
}

/// `git status` started in the background for a repository directory.
pub struct Probe {
    dot_git: PathBuf,
    status: Option<Running>,
    timeout_at: Instant,
}

/// Start reading the repository containing `cwd`; `None` outside any repository.
pub fn start(cwd: &Path) -> Option<Probe> {
    let dot_git = cwd
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|dot_git| dot_git.exists())?;
    let timeout_at = Instant::now() + TIMEOUT;
    let status = Running::spawn(
        Command::new("git")
            .args([
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "--branch",
            ])
            .current_dir(cwd),
    );
    Some(Probe {
        dot_git,
        status,
        timeout_at,
    })
}

impl Probe {
    /// The status, waiting until the timeout or `deadline`, whichever is first.
    pub fn finish(self, deadline: Instant) -> Git {
        self.status
            .and_then(|status| status.output_by(deadline.min(self.timeout_at)))
            .and_then(|output| parse_status(&String::from_utf8_lossy(&output)))
            .map_or_else(|| Git::Unknown(read_head(&self.dot_git)), Git::Known)
    }
}

/// The `git status --porcelain=v2 --branch` headers, and whether any entry
/// follows them. `None` when the branch headers are missing.
pub fn parse_status(output: &str) -> Option<Status> {
    let (mut oid, mut branch, mut ahead, mut behind, mut dirty) = (None, None, 0, 0, false);
    for line in output.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            dirty |= !line.is_empty();
            continue;
        };
        match header.split_once(' ') {
            Some(("branch.oid", value)) => oid = Some(value),
            Some(("branch.head", value)) => branch = Some(value),
            Some(("branch.ab", value)) => {
                for count in value.split(' ') {
                    if let Some(count) = count.strip_prefix('+') {
                        ahead = count.parse().ok()?;
                    } else if let Some(count) = count.strip_prefix('-') {
                        behind = count.parse().ok()?;
                    }
                }
            }
            _ => {}
        }
    }
    let head = match branch? {
        "(detached)" => Head::Detached(short_hash(oid?)?),
        name => Head::Branch(name.to_owned()),
    };
    Some(Status {
        head,
        dirty,
        ahead,
        behind,
    })
}

/// The head named by `HEAD`, following a `.git` file to its git directory as
/// linked worktrees and submodules have.
fn read_head(dot_git: &Path) -> Option<Head> {
    let git_dir = if dot_git.is_file() {
        let link = std::fs::read_to_string(dot_git).ok()?;
        let target = link.trim_end().strip_prefix("gitdir: ")?;
        dot_git.parent()?.join(target)
    } else {
        dot_git.to_owned()
    };
    parse_head(&std::fs::read_to_string(git_dir.join("HEAD")).ok()?)
}

fn parse_head(head: &str) -> Option<Head> {
    let head = head.trim_end();
    match head.strip_prefix("ref: ") {
        Some(target) => Some(Head::Branch(
            target
                .strip_prefix("refs/heads/")
                .unwrap_or(target)
                .to_owned(),
        )),
        None => short_hash(head).map(Head::Detached),
    }
}

fn short_hash(oid: &str) -> Option<String> {
    (oid.len() > SHORT_HASH && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| oid[..SHORT_HASH].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "7d4a1c2e9b0f3a5d6c8e1f2a3b4c5d6e7f8a9b0c";

    fn status(head: Head, dirty: bool, ahead: u32, behind: u32) -> Option<Status> {
        Some(Status {
            head,
            dirty,
            ahead,
            behind,
        })
    }

    fn main_branch() -> Head {
        Head::Branch("main".into())
    }

    #[test]
    fn reads_branch_tracking_and_dirtiness() {
        let clean = format!("# branch.oid {OID}\n# branch.head main\n");
        assert_eq!(parse_status(&clean), status(main_branch(), false, 0, 0));

        let tracking = format!(
            "# branch.oid {OID}\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n"
        );
        assert_eq!(parse_status(&tracking), status(main_branch(), false, 2, 1));

        for entry in [
            "1 .M N... 100644 100644 100644 aaaa bbbb notes.txt",
            "? scratch.txt",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc both.txt",
        ] {
            let dirty = format!("{clean}{entry}\n");
            assert_eq!(
                parse_status(&dirty),
                status(main_branch(), true, 0, 0),
                "{entry}"
            );
        }
    }

    #[test]
    fn reads_detached_and_unborn_heads() {
        let detached = format!("# branch.oid {OID}\n# branch.head (detached)\n");
        assert_eq!(
            parse_status(&detached),
            status(Head::Detached("7d4a1c2".into()), false, 0, 0)
        );
        let unborn = "# branch.oid (initial)\n# branch.head main\n";
        assert_eq!(parse_status(unborn), status(main_branch(), false, 0, 0));
    }

    #[test]
    fn rejects_output_without_branch_headers() {
        assert_eq!(parse_status(""), None);
        assert_eq!(parse_status("? scratch.txt\n"), None);
        assert_eq!(
            parse_status("# branch.oid (initial)\n# branch.head (detached)\n"),
            None
        );
    }

    #[test]
    fn reads_head_files() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/x\n"),
            Some(Head::Branch("feature/x".into()))
        );
        assert_eq!(
            parse_head(&format!("{OID}\n")),
            Some(Head::Detached("7d4a1c2".into()))
        );
        assert_eq!(parse_head("garbage\n"), None);
    }

    #[test]
    fn follows_a_dot_git_file_to_the_git_directory() {
        let root = tempfile::tempdir().expect("temp dir");
        let git_dir = root.path().join("main.git/worktrees/wt");
        std::fs::create_dir_all(&git_dir).expect("git dir");
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/wt\n").expect("HEAD");
        let worktree = root.path().join("wt");
        std::fs::create_dir(&worktree).expect("worktree");
        std::fs::write(worktree.join(".git"), "gitdir: ../main.git/worktrees/wt\n")
            .expect(".git file");
        assert_eq!(
            read_head(&worktree.join(".git")),
            Some(Head::Branch("wt".into()))
        );
    }
}
