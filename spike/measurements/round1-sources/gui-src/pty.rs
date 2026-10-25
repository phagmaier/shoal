//! Spawn the shell in a PTY and stream its output to the UI thread.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use futures::SinkExt;
use futures::channel::mpsc;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

pub struct Pty {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Writer,
    #[allow(dead_code, reason = "keeps the child handle alive")]
    pub child: Box<dyn Child + Send + Sync>,
}

impl Pty {
    pub fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

/// Start `$SPIKE_CMD` under bash, or an interactive login bash.
/// Output arrives on the returned channel in chunks of up to 64 KiB; the
/// channel is bounded, so a slow UI applies backpressure to the shell.
pub fn spawn(cols: u16, rows: u16) -> anyhow_lite::Result<(Pty, mpsc::Receiver<Vec<u8>>)> {
    let pair = native_pty_system().openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::new("bash");
    match std::env::var("SPIKE_CMD") {
        Ok(script) => cmd.args(["-c", &script]),
        Err(_) => cmd.args(["-l"]),
    }
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "shoal-spike");
    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let writer: Writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let (mut tx, rx) = mpsc::channel::<Vec<u8>>(4);
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            // EIO when the child exits is end of file.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || futures::executor::block_on(tx.send(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        })?;
    Ok((
        Pty {
            master: pair.master,
            writer,
            child,
        },
        rx,
    ))
}

/// The spike has no anyhow; a boxed error is enough.
pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
