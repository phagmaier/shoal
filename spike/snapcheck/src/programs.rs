//! Real programs in a real PTY. Terminal A plays the daemon: it is fed the
//! program's output and answers its queries. Restored terminals play mirrors.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::{Outcome, check, new_terminal};

struct Program {
    name: &'static str,
    argv: Vec<String>,
    /// Keys sent before the snapshot.
    prep: &'static [u8],
    /// Keys sent after the snapshot, so the mirror sees incremental output.
    keys: &'static [u8],
}

fn sh(name: &'static str, cmd: &str, prep: &'static [u8], keys: &'static [u8]) -> Program {
    Program {
        name,
        argv: vec!["bash".into(), "-c".into(), cmd.into()],
        prep,
        keys,
    }
}

const COLS: u16 = 120;
const ROWS: u16 = 40;

/// Collect output until it has been quiet for `quiet`, or `max` passes.
fn drain(rx: &Receiver<Vec<u8>>, quiet: Duration, max: Duration) -> Vec<u8> {
    let start = Instant::now();
    let mut out = Vec::new();
    while start.elapsed() < max {
        match rx.recv_timeout(quiet) {
            Ok(bytes) => out.extend_from_slice(&bytes),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    out
}

fn run(p: &Program, fixtures: &std::path::Path) -> Option<Outcome> {
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .ok()?;
    let mut cmd = CommandBuilder::new(&p.argv[0]);
    cmd.args(&p.argv[1..]);
    cmd.cwd(fixtures);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let mut child = pty.slave.spawn_command(cmd).ok()?;
    drop(pty.slave);

    let mut reader = pty.master.try_clone_reader().ok()?;
    let writer = Arc::new(Mutex::new(pty.master.take_writer().ok()?));
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut a = new_terminal(COLS, ROWS);
    let replies = Arc::clone(&writer);
    a.on_pty_write(move |_, data| {
        if let Ok(mut w) = replies.lock() {
            let _ = w.write_all(data);
        }
    })
    .ok()?;

    // Let the program start and draw, answering its queries as we go.
    for keys in [&b""[..], p.prep] {
        if let Ok(mut w) = writer.lock() {
            let _ = w.write_all(keys);
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            let chunk = drain(&rx, Duration::from_millis(400), Duration::from_millis(400));
            if chunk.is_empty() {
                break;
            }
            a.vt_write(&chunk);
        }
    }

    // The snapshot happens inside `check`. Then the keys go in, and their
    // output streams into A (which answers queries) and every restored copy.
    // A NUL in `keys` means "pause", so Esc isn't read as Alt+next key.
    let mut parts = p.keys.split(|b| *b == 0);
    let mut pending_part = true;
    let outcome = check(p.name, &mut a, || {
        loop {
            if pending_part {
                pending_part = false;
                let part = parts.next()?;
                if let Ok(mut w) = writer.lock() {
                    let _ = w.write_all(part);
                }
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(bytes) => return Some(bytes),
                Err(_) => pending_part = true,
            }
        }
    });

    let _ = child.kill();
    let _ = child.wait();
    Some(outcome)
}

fn fixtures() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("snapcheck-fixtures");
    let _ = std::fs::create_dir_all(&dir);
    let mut src = String::new();
    for i in 0..300 {
        src.push_str(&format!(
            "fn item_{i}(x: u32) -> u32 {{\n    // comment {i}\n    let s = \"string {i}\";\n    x + {i}\n}}\n"
        ));
    }
    let _ = std::fs::write(dir.join("sample.rs"), src);
    let mut colored = String::new();
    for i in 0..500 {
        colored.push_str(&format!(
            "\x1b[3{}mline {i}\x1b[0m \x1b[1mbold\x1b[0m plain text 漢字\n",
            i % 8
        ));
    }
    let _ = std::fs::write(dir.join("colored.txt"), colored);
    if !dir.join(".git").exists() {
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status();
        let _ = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .status();
        let _ = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ])
            .current_dir(&dir)
            .status();
    }
    dir
}

pub fn run_all() -> Vec<Outcome> {
    let dir = fixtures();
    let programs = [
        sh(
            "nvim --clean, syntax on, then quit",
            "seq 1 200; exec nvim --clean +'syntax on' +'set number cursorline' +120 sample.rs",
            b"",
            b"jjjjwwiTYPED\x1b\0:qa!\r",
        ),
        sh(
            "less -R, then quit",
            "seq 1 50; less -R colored.txt; echo after-less",
            b"",
            b"jjjj q",
        ),
        sh(
            "btop, then quit",
            "seq 1 50; btop; echo after-btop",
            b"",
            b"\x1b[B\x1b[B\0q",
        ),
        sh(
            "fzf (inline, --height 40%), then cancel",
            "seq 1 100; seq 1 1000 | fzf --height 40%; echo after-fzf",
            b"",
            b"12\x1b",
        ),
        sh(
            "lazygit, then quit",
            "seq 1 50; lazygit; echo after-lazygit",
            b"",
            b"jq",
        ),
        sh(
            "interactive bash with prompt marks",
            "exec bash --norc --noprofile -i",
            b"PS1=$'\\e]133;A\\a\\e[32m$ \\e[0m\\e]133;B\\a'\r ls -la --color=always /\r",
            b"echo done\r",
        ),
    ];
    programs.iter().filter_map(|p| run(p, &dir)).collect()
}
