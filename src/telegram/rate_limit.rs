//! Rate limiting for Telegram API calls
//!
//! Telegram has rate limits that vary by operation. This module
//! provides a token bucket rate limiter to stay within limits.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;

/// Rate limiter using token bucket algorithm
pub struct RateLimiter {
    /// Maximum concurrent operations
    concurrency: Semaphore,
    /// Minimum delay between operations (microseconds)
    min_delay_us: AtomicU64,
    /// Last operation timestamp
    last_op: parking_lot::Mutex<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter
    ///
    /// # Arguments
    /// * `max_concurrent` - Maximum concurrent operations
    /// * `ops_per_second` - Target operations per second
    pub fn new(max_concurrent: usize, ops_per_second: f64) -> Self {
        let min_delay_us = if ops_per_second > 0.0 {
            (1_000_000.0 / ops_per_second) as u64
        } else {
            0
        };

        RateLimiter {
            concurrency: Semaphore::new(max_concurrent),
            min_delay_us: AtomicU64::new(min_delay_us),
            last_op: parking_lot::Mutex::new(Instant::now()),
        }
    }

    /// Create an unlimited rate limiter (for testing)
    pub fn unlimited() -> Self {
        RateLimiter {
            concurrency: Semaphore::new(100),
            min_delay_us: AtomicU64::new(0),
            last_op: parking_lot::Mutex::new(Instant::now()),
        }
    }

    /// Acquire permission to perform an operation
    pub async fn acquire(&self) -> RateLimitGuard<'_> {
        // Acquire concurrency permit
        let permit = self.concurrency.acquire().await.unwrap();

        // Enforce minimum delay
        let min_delay = Duration::from_micros(self.min_delay_us.load(Ordering::Relaxed));
        if !min_delay.is_zero() {
            let mut last_op = self.last_op.lock();
            let elapsed = last_op.elapsed();

            if elapsed < min_delay {
                let wait_time = min_delay - elapsed;
                drop(last_op); // Release lock while sleeping
                sleep(wait_time).await;
                last_op = self.last_op.lock();
            }

            *last_op = Instant::now();
        }

        RateLimitGuard { _permit: permit }
    }

    /// Temporarily increase delay (for backoff)
    pub fn increase_delay(&self, factor: f64) {
        let current = self.min_delay_us.load(Ordering::Relaxed);
        let new_delay = ((current as f64) * factor) as u64;
        self.min_delay_us.store(new_delay.min(10_000_000), Ordering::Relaxed); // Cap at 10s
    }

    /// Reset delay to normal
    pub fn reset_delay(&self, ops_per_second: f64) {
        let min_delay_us = if ops_per_second > 0.0 {
            (1_000_000.0 / ops_per_second) as u64
        } else {
            0
        };
        self.min_delay_us.store(min_delay_us, Ordering::Relaxed);
    }
}

/// Guard that releases rate limit permit on drop
pub struct RateLimitGuard<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
}

/// Exponential backoff helper
pub struct ExponentialBackoff {
    base_delay: Duration,
    max_delay: Duration,
    max_attempts: u32,
    current_attempt: u32,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff
    pub fn new(base_delay_ms: u64, max_attempts: u32) -> Self {
        ExponentialBackoff {
            base_delay: Duration::from_millis(base_delay_ms),
            max_delay: Duration::from_secs(60),
            max_attempts,
            current_attempt: 0,
        }
    }

    /// Get the next delay, or None if max attempts reached
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_attempt >= self.max_attempts {
            return None;
        }

        let delay = self.base_delay * 2u32.saturating_pow(self.current_attempt);
        self.current_attempt += 1;

        Some(delay.min(self.max_delay))
    }

    /// Reset the backoff
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }

    /// Check if we have attempts remaining
    pub fn has_attempts(&self) -> bool {
        self.current_attempt < self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_concurrency() {
        let limiter = RateLimiter::new(2, 100.0);

        // Should be able to acquire 2 permits
        let _g1 = limiter.acquire().await;
        let _g2 = limiter.acquire().await;

        // Third should block (but we can't easily test that without timeout)
    }

    #[test]
    fn test_exponential_backoff() {
        let mut backoff = ExponentialBackoff::new(100, 3);

        let d1 = backoff.next_delay().unwrap();
        let d2 = backoff.next_delay().unwrap();
        let d3 = backoff.next_delay().unwrap();
        let d4 = backoff.next_delay();

        assert_eq!(d1, Duration::from_millis(100));
        assert_eq!(d2, Duration::from_millis(200));
        assert_eq!(d3, Duration::from_millis(400));
        assert!(d4.is_none());
    }

    #[test]
    fn test_backoff_reset() {
        let mut backoff = ExponentialBackoff::new(100, 2);

        backoff.next_delay();
        backoff.next_delay();
        assert!(backoff.next_delay().is_none());

        backoff.reset();
        assert!(backoff.next_delay().is_some());
    }
}
