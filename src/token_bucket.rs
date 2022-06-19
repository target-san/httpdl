use std::cmp;
use std::time::Instant;

pub struct TokenBucket {
    fill_rate: usize,
    capacity:  usize,
    remaining: f64,
    timestamp: Instant,
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
        let delta_fill = ((delta.as_secs() as f64) + (delta.subsec_nanos() as f64) / 1_000_000_000f64) * (self.fill_rate as f64);
        self.remaining = (self.remaining + delta_fill).min(self.capacity as f64);
        // 2. Take as much as possible from bucket, but no more than is present there
        let taken = cmp::min(self.remaining.floor() as usize, amount);
        self.remaining = (self.remaining - (taken as f64)).max(0f64);
        return taken;
    }
}
