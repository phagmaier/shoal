use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Instant,
};

/// A child process whose standard output is collected on a reader thread, so
/// waiting for it can stop at a deadline.
pub struct Running {
    child: Child,
    output: Receiver<Vec<u8>>,
}

impl Running {
    /// Start `command` with no input and its errors discarded; `None` when it
    /// cannot start, such as when the program is not installed.
    pub fn spawn(command: &mut Command) -> Option<Self> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (send, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            // A read error leaves the output short, which callers treat as unusable.
            let _ = stdout.read_to_end(&mut bytes);
            let _ = send.send(bytes);
        });
        Some(Self { child, output })
    }

    /// Everything the child printed, once it closes its output, or `None` if
    /// `deadline` passes first. A child still running at the deadline is killed.
    pub fn output_by(mut self, deadline: Instant) -> Option<Vec<u8>> {
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.output.recv_timeout(wait) {
            Ok(bytes) => {
                // Reap it if it has exited; one still closing down is reaped by
                // init once `shoal` exits, a moment from now.
                let _ = self.child.try_wait();
                Some(bytes)
            }
            Err(_) => {
                let _ = self.child.kill();
                None
            }
        }
    }
}
