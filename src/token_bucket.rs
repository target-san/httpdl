use std::cmp;
use std::time::{Instant, Duration};

pub struct TokenBucket {
    fill_rate: usize,
    capacity:  usize,
    remaining: f64,
    timestamp: Instant,
}

fn duration_seconds(d: Duration) -> f64 {
    (d.as_secs() as f64) + (d.subsec_nanos() as f64) / 1_000_000_000f64
}

impl TokenBucket {
    pub fn new(rate: usize) -> TokenBucket {
        TokenBucket::with_capacity(rate, rate)
    }

    pub fn with_capacity(rate: usize, capacity: usize) -> TokenBucket {
        TokenBucket {
            fill_rate: rate,
            capacity:  capacity,
            remaining: 0f64,
            timestamp: Instant::now(),
        }
    }

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
        return taken;
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
