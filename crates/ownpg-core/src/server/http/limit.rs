use std::num::NonZeroU32;
use std::time::Duration;

use governor::clock::Clock;
use governor::{DefaultKeyedRateLimiter, Quota};

pub struct Limiter {
    limiter: DefaultKeyedRateLimiter<String>,
    calls_per_minute: u32,
}

impl Limiter {
    #[must_use]
    pub fn new(calls_per_minute: u32) -> Self {
        let burst = NonZeroU32::new(calls_per_minute.max(1)).unwrap_or(NonZeroU32::MIN);
        Self {
            limiter: DefaultKeyedRateLimiter::keyed(Quota::per_minute(burst)),
            calls_per_minute: calls_per_minute.max(1),
        }
    }

    #[must_use]
    pub const fn calls_per_minute(&self) -> u32 {
        self.calls_per_minute
    }

    pub fn check(&self, key: &str) -> Result<(), Duration> {
        match self.limiter.check_key(&key.to_owned()) {
            Ok(()) => Ok(()),
            Err(not_until) => {
                let wait = not_until.wait_time_from(self.limiter.clock().now());
                Err(wait)
            }
        }
    }

    pub fn forget_idle(&self) {
        self.limiter.retain_recent();
    }
}

impl std::fmt::Debug for Limiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limiter")
            .field("calls_per_minute", &self.calls_per_minute)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sixty_first_call_in_a_minute_waits_and_keys_are_independent() {
        let limiter = Limiter::new(60);
        for _ in 0..60 {
            assert!(limiter.check("a").is_ok());
        }
        let wait = limiter.check("a").unwrap_err();
        assert!(wait > Duration::ZERO && wait <= Duration::from_secs(1));
        assert!(limiter.check("b").is_ok());
    }
}
