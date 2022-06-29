use std::cmp;
use std::time::{Duration, Instant};

/// A bucket of tokens which renews itself with time
///
/// Used to generate time-constrained quota for some repeatable process,
/// like copying data from one stream to another
pub struct TokenBucket {
    /// How many tokens are generated per second
    fill_rate: usize,
    /// Maximum number of tokens in bucket
    capacity: usize,
    /// How many tokens remain in bucket, with fraction
    remaining: f64,
    /// Last time tokens were taken from bucket
    timestamp: Instant,
}
/// Convert time duration to seconds, with nanoseconds as fraction
fn duration_seconds(d: Duration) -> f64 {
    (d.as_secs() as f64) + (d.subsec_nanos() as f64) / 1_000_000_000f64
}

impl TokenBucket {
    /// Creates new token bucket, with fill rate and capacity set to specified value
    ///
    /// # Arguments
    /// * rate - value for both fill rate and capacity
    pub fn new(rate: usize) -> TokenBucket {
        TokenBucket::with_capacity(rate, rate)
    }
    /// Creates new token bucket with specified fill rate and capacity
    ///
    /// # Arguments
    /// * rate - how many tokens are generated per second;
    ///     set to 0 to make bucket unlimited
    /// * capacity - how many tokens can bucket hold; can be 0 if fill rate is 0 too
    ///
    /// # Panics
    /// Panics if rate argument != 0 while capacity == 0
    ///
    pub fn with_capacity(rate: usize, capacity: usize) -> TokenBucket {
        if rate != 0 && capacity == 0 {
            panic!("Cannot construct token bucket with nonzero rate and zero capacity");
        }
        TokenBucket {
            fill_rate: rate,
            capacity,
            remaining: 0f64,
            timestamp: Instant::now(),
        }
    }
    /// Attempts to take specified amount of tokens from bucket
    ///
    /// # Arguments
    /// * amount - try to get this many tokens
    ///
    /// # Returns
    /// Number of tokens actually retrieved
    ///
    /// If fill rate is zero, returns requested amount right away.
    /// Otherwise, does following:
    /// * Computes how much time has passed since previous call (or instance construction)
    /// * Refills bucket storage by fill rate multiplied by delta time, capped by capacity
    /// * Takes requested amount, but no more than remaining tokens and returns it
    pub fn take(&mut self, amount: usize) -> usize {
        // 0. For zero fillrate, treat this bucket as infinite
        if self.fill_rate == 0 {
            return amount;
        }
        // 1. Add to bucket rate / delta
        let delta = {
            let now = Instant::now();
            now - std::mem::replace(&mut self.timestamp, now)
        };
        let delta_fill = duration_seconds(delta) * (self.fill_rate as f64);
        self.remaining = (self.remaining + delta_fill).min(self.capacity as f64);
        // 2. Take as much as possible from bucket, but no more than is present there
        let taken = cmp::min(self.remaining.floor() as usize, amount);
        self.remaining = (self.remaining - (taken as f64)).max(0f64);
        taken
    }
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::Duration;

    use super::TokenBucket;

    fn get_random(limit: usize) -> usize {
        use rand::Rng;
        rand::thread_rng().gen_range(0..limit)
    }

    #[test]
    fn test_new() {
        let rate = get_random(1_000_000);
        let tb = TokenBucket::new(rate);
        assert_eq!(tb.capacity, rate);
        assert_eq!(tb.fill_rate, rate);
    }

    #[test]
    fn test_with_capacity() {
        let cap = get_random(1_000_000);
        let rate = get_random(1_000_000);
        let tb = TokenBucket::with_capacity(rate, cap);

        assert_eq!(tb.capacity, cap);
        assert_eq!(tb.fill_rate, rate);
    }

    #[test]
    fn test_take_simple() {
        let rate = 1_000;
        let wait_ms = get_random(1_000);
        let mut tb = TokenBucket::new(rate);

        let before = tb.timestamp;

        sleep(Duration::from_millis(wait_ms as u64));
        let taken = tb.take(wait_ms / 2);
        assert_eq!(taken, wait_ms / 2);

        let after = tb.timestamp;

        let delta = super::duration_seconds(after - before) * (rate as f64);

        assert_eq!((delta - tb.remaining).floor() as usize, taken);
    }
}
