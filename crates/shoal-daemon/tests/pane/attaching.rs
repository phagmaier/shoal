use super::*;
use shoal_daemon::pane::{AttachError, Attachment, Delivery};
use shoal_term::{SnapshotError, Terminal};
use std::sync::Arc;
use tokio::sync::{Notify, oneshot};

fn attach(pane: &Pane) -> oneshot::Receiver<Result<Attachment, AttachError>> {
    pane.attach(Arc::new(Notify::new()))
}

fn answer<T>(mut receiver: oneshot::Receiver<T>) -> T {
    eventually("pane reply", || match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(oneshot::error::TryRecvError::Empty) => None,
        Err(error) => panic!("pane reply closed: {error}"),
    })
}

fn long_osc(harness: &mut Harness, tail: &str) -> Pane {
    let script = format!("stty -echo; printf '\\033]0;'; printf '%05000d' 0; {tail}");
    let pane = harness.spawn(&["bash", "-c", &script]);
    eventually("unfinished OSC in the terminal", || {
        matches!(answer(pane.snapshot()), Err(SnapshotError::MidSequence)).then_some(())
    });
    pane
}

#[test]
fn waiting_attach_splits_at_first_ground_before_another_long_sequence() {
    let mut harness = Harness::new();
    let pane = long_osc(
        &mut harness,
        "read -r gate; printf '\\007after-ground\\033]0;'; printf '%05000d' 0; read -r gate; printf '\\007\\r\\nfinished\\r\\n'",
    );
    let mut pending = attach(&pane);
    // Commands are FIFO: this capture fences the waiting attach registration.
    answer(pane.capture(CaptureFormat::Text, false)).unwrap();
    assert!(matches!(
        pending.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    pane.write(b"go\n".to_vec());
    let mut attachment = answer(pending).unwrap();
    let snapshot = &attachment.snapshot;
    let mut mirror = Terminal::restore(
        &snapshot.bytes,
        snapshot.size.cols,
        snapshot.size.rows,
        10000,
    )
    .unwrap();
    assert!(mirror.is_ground().unwrap());
    assert!(
        !String::from_utf8(mirror.capture(CaptureFormat::Text, false).unwrap())
            .unwrap()
            .contains("after-ground")
    );
    eventually("second unfinished sequence", || {
        matches!(answer(pane.snapshot()), Err(SnapshotError::MidSequence)).then_some(())
    });
    pane.write(b"go\n".to_vec());
    harness.exited(&pane);
    let end = answer(pane.snapshot()).unwrap();
    let mut seq = snapshot.seq;
    let mut bytes = Vec::new();
    while seq < end.seq {
        let item = eventually("live item after attach", || {
            attachment.items.try_recv().ok()
        });
        let Delivery::Item(Item::Output {
            seq: next,
            bytes: output,
        }) = item
        else {
            panic!("unexpected {item:?}");
        };
        assert_eq!(next, seq + 1);
        seq = next;
        mirror.feed(&output);
        bytes.extend_from_slice(&output);
    }
    let expected = [
        b"after-ground\x1b]0;".as_slice(),
        "0".repeat(5000).as_bytes(),
        b"\x07\r\r\nfinished\r\r\n",
    ]
    .concat();
    assert_eq!(
        bytes, expected,
        "exactly the bytes after the first terminator"
    );
    assert_eq!(
        String::from_utf8(mirror.capture(CaptureFormat::Text, false).unwrap()).unwrap(),
        screen(&pane)
    );
}

#[test]
fn unfinished_attach_times_out_while_the_shell_keeps_running_and_retry_works() {
    let mut harness = Harness::new();
    let pane = long_osc(
        &mut harness,
        "read -r gate; : > kept-running; read -r gate; printf '\\007done\\r\\n'",
    );
    let started = Instant::now();
    let pending = attach(&pane);
    answer(pane.capture(CaptureFormat::Text, false)).unwrap();
    pane.write(b"go\n".to_vec());
    eventually("shell progress during pending attach", || {
        harness
            .env
            .home()
            .join("kept-running")
            .exists()
            .then_some(())
    });
    assert!(matches!(answer(pending), Err(AttachError::MidSequence)));
    assert!(started.elapsed() >= Duration::from_millis(1900));
    assert!(started.elapsed() < Duration::from_millis(3500));
    pane.write(b"go\n".to_vec());
    harness.exited(&pane);
    assert!(answer(attach(&pane)).is_ok());
}

#[test]
fn overflowing_mid_sequence_resync_expires_and_detaches_only_that_subscriber() {
    let mut harness = Harness::new();
    let pane = long_osc(
        &mut harness,
        "read -r gate; head -c 10000000 /dev/zero | tr '\\0' x; : > flooded; read -r gate; printf '\\007done\\r\\n'",
    );
    let (_, mut slow) = pane.subscribe();
    answer(pane.capture(CaptureFormat::Text, false)).unwrap();
    pane.write(b"go\n".to_vec());
    eventually("shell to finish flooding", || {
        harness.env.home().join("flooded").exists().then_some(())
    });
    // A pane command fences the last read without draining this subscriber.
    eventually("resync detach", || {
        while let Ok(delivery) = slow.try_recv() {
            if matches!(delivery, Delivery::Detached) {
                return Some(());
            }
        }
        None
    });
    pane.write(b"go\n".to_vec());
    harness.exited(&pane);
    assert!(screen(&pane).contains("done"));
    assert!(answer(attach(&pane)).is_ok());
}
