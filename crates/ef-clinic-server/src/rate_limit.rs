//! Per-IP sliding-window ingress rate limiter.
//!
//! Cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::{BTreeMap, VecDeque};

/// Configuration for the per-IP sliding-window ingress rate limiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Maximum requests permitted per IP within `window_secs`.
    pub max_requests: usize,
    /// Sliding window width in seconds.
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 120,
            window_secs: 60,
        }
    }
}

/// Outcome of evaluating a request against [`IpSlidingWindowLimiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed { remaining: usize },
    Exceeded { retry_after_secs: u64 },
}

/// Single-owner per-IP sliding-window ingress rate limiter.
#[derive(Debug, Clone)]
pub struct IpSlidingWindowLimiter {
    config: RateLimitConfig,
    windows: BTreeMap<String, VecDeque<u64>>,
}

impl IpSlidingWindowLimiter {
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            windows: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> RateLimitConfig {
        self.config
    }

    /// Updates the rate-limit configuration and clears stale counters.
    pub fn reconfigure(&mut self, config: RateLimitConfig) {
        self.config = config;
        self.windows.clear();
    }

    /// Evaluates and records one request from `client_ip` at `now_secs`.
    pub fn check_and_record(&mut self, client_ip: &str, now_secs: u64) -> RateLimitDecision {
        if self.config.max_requests == 0 {
            return RateLimitDecision::Exceeded {
                retry_after_secs: self.config.window_secs.max(1),
            };
        }

        let window_secs = self.config.window_secs.max(1);
        let queue = self.windows.entry(client_ip.to_owned()).or_default();

        while let Some(&oldest) = queue.front() {
            if now_secs >= oldest.saturating_add(window_secs) {
                let _ = queue.pop_front();
            } else {
                break;
            }
        }

        if queue.len() >= self.config.max_requests {
            let retry_after_secs = queue
                .front()
                .copied()
                .map_or(window_secs, |oldest| {
                    oldest.saturating_add(window_secs).saturating_sub(now_secs)
                })
                .max(1);
            return RateLimitDecision::Exceeded { retry_after_secs };
        }

        queue.push_back(now_secs);
        let remaining = self.config.max_requests.saturating_sub(queue.len());
        RateLimitDecision::Allowed { remaining }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_window_enforces_limit_and_recovers_after_window() {
        let mut limiter = IpSlidingWindowLimiter::new(RateLimitConfig {
            max_requests: 3,
            window_secs: 10,
        });
        assert_eq!(
            limiter.check_and_record("127.0.0.1", 100),
            RateLimitDecision::Allowed { remaining: 2 }
        );
        assert_eq!(
            limiter.check_and_record("127.0.0.1", 102),
            RateLimitDecision::Allowed { remaining: 1 }
        );
        assert_eq!(
            limiter.check_and_record("127.0.0.1", 104),
            RateLimitDecision::Allowed { remaining: 0 }
        );
        assert_eq!(
            limiter.check_and_record("127.0.0.1", 105),
            RateLimitDecision::Exceeded {
                retry_after_secs: 5
            }
        );
        // Independent IP is unaffected.
        assert_eq!(
            limiter.check_and_record("10.0.0.2", 105),
            RateLimitDecision::Allowed { remaining: 2 }
        );
        // After oldest request expires at t=110, request is allowed again.
        assert_eq!(
            limiter.check_and_record("127.0.0.1", 110),
            RateLimitDecision::Allowed { remaining: 0 }
        );
    }
}
