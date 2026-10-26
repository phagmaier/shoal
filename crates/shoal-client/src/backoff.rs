//! Reconnect delays: 100 ms, doubling to at most 2 s (M1 spec §4.1, T20).

use std::time::Duration;

const FIRST: Duration = Duration::from_millis(100);
const MAX: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
pub(crate) struct Backoff {
    next: Option<Duration>,
}

impl Backoff {
    /// The delay before the next attempt.
    pub fn next(&mut self) -> Duration {
        let delay = self.next.unwrap_or(FIRST);
        self.next = Some((delay * 2).min(MAX));
        delay
    }

    /// After a successful `hello`, start again from 100 ms.
    pub fn reset(&mut self) {
        self.next = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_from_a_tenth_of_a_second_to_two_and_resets() {
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..7).map(|_| backoff.next().as_millis()).collect();
        assert_eq!(delays, [100, 200, 400, 800, 1600, 2000, 2000]);
        backoff.reset();
        assert_eq!(backoff.next(), FIRST);
    }
}
