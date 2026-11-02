//! Run-wide dispatch spacing after the service limits requests. Every
//! request, including a validation retry, acquires its turn immediately
//! before sending. Queued waiters recheck the shared deadline after waking.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

const INITIAL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_WAIT: Duration = Duration::from_secs(20);

pub(crate) struct Throttle {
    state: Mutex<State>,
}

struct State {
    generation: u64,
    interval: Duration,
    next: Instant,
    last_sent: Option<Instant>,
}

impl State {
    fn new(now: Instant) -> Self {
        Self {
            generation: 0,
            interval: INITIAL_INTERVAL,
            next: now,
            last_sent: None,
        }
    }

    fn acquire(&mut self, now: Instant) -> Result<u64, Instant> {
        if now < self.next {
            return Err(self.next);
        }
        self.last_sent = Some(now);
        self.next = now + self.interval;
        Ok(self.generation)
    }

    fn limited(&mut self, generation: u64, now: Instant, wait: Duration) {
        // Responses to requests sent before the same slowdown belong to
        // one burst: only a newly sent request can double the interval again.
        if generation == self.generation {
            self.generation = self.generation.saturating_add(1);
            self.interval = (self.interval * 2).clamp(INITIAL_INTERVAL, MAX_INTERVAL);
        }
        self.next = self.next.max(now + wait);
        if let Some(last_sent) = self.last_sent {
            self.next = self.next.max(last_sent + self.interval);
        }
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::new(Instant::now())),
        }
    }
}

impl Throttle {
    pub(crate) async fn acquire(&self) -> u64 {
        loop {
            let turn = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .acquire(Instant::now());
            match turn {
                Ok(generation) => return generation,
                Err(until) => tokio::time::sleep_until(until).await,
            }
        }
    }

    /// Blocks all new sends at once, before the scheduler handles the
    /// failed batch. The learned interval is kept for the rest of the run.
    pub(crate) fn limited(
        &self,
        generation: u64,
        retry_after: Option<Duration>,
    ) -> (Duration, Duration) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.limited(generation, now, retry_after.unwrap_or(DEFAULT_WAIT));
        (state.next.saturating_duration_since(now), state.interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_spaced_even_before_the_first_rate_limit() {
        let now = Instant::now();
        let mut state = State::new(now);
        assert_eq!(state.acquire(now), Ok(0));
        assert_eq!(state.acquire(now), Err(now + INITIAL_INTERVAL));
        assert_eq!(state.acquire(now + INITIAL_INTERVAL), Ok(0));
    }

    #[test]
    fn a_limit_blocks_already_waiting_requests_and_spaces_later_sends() {
        let now = Instant::now();
        let mut state = State::new(now);
        let first = state.acquire(now).expect("first request");
        state.limited(first, now, Duration::from_secs(40));
        assert_eq!(
            state.acquire(now + Duration::from_secs(39)),
            Err(now + Duration::from_secs(40))
        );
        assert_eq!(state.acquire(now + Duration::from_secs(40)), Ok(1));
        assert_eq!(
            state.acquire(now + Duration::from_secs(40)),
            Err(now + Duration::from_secs(41))
        );
        assert_eq!(state.acquire(now + Duration::from_secs(41)), Ok(1));
    }

    #[test]
    fn one_burst_does_not_multiply_the_interval_for_each_old_response() {
        let now = Instant::now();
        let mut state = State::new(now);
        let old = state.acquire(now).expect("request");
        state.limited(old, now, Duration::from_secs(20));
        for _ in 0..16 {
            state.limited(old, now, Duration::from_secs(30));
        }
        assert_eq!(state.interval, INITIAL_INTERVAL * 2);
        let fresh = state
            .acquire(now + Duration::from_secs(30))
            .expect("after cooldown");
        state.limited(fresh, now + Duration::from_secs(30), Duration::ZERO);
        assert_eq!(state.interval, Duration::from_secs(2));
        state.limited(old, now + Duration::from_secs(30), Duration::ZERO);
        assert_eq!(state.interval, Duration::from_secs(2));
    }

    #[test]
    fn repeated_limits_keep_a_bounded_dispatch_interval() {
        let mut now = Instant::now();
        let mut state = State::new(now);
        for _ in 0..100 {
            let generation = state.acquire(now).expect("request");
            state.limited(generation, now, Duration::ZERO);
            now = state.next;
        }
        assert_eq!(state.interval, MAX_INTERVAL);
    }

    #[tokio::test]
    async fn a_waiting_request_can_be_cancelled_without_consuming_a_turn() {
        let throttle = Throttle::default();
        let first = throttle.acquire().await;
        throttle.limited(first, Some(Duration::from_secs(60)));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), throttle.acquire())
                .await
                .is_err()
        );
        let state = throttle
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(state.generation, 1);
        assert_eq!(state.interval, INITIAL_INTERVAL * 2);
    }
}
