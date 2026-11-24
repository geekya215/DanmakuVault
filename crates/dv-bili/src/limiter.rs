//! Token bucket rate limiting.
//!
//! This only limits how many requests are issued per second; the concurrency cap is a separate
//! concern, so do not conflate the two.

use std::sync::Mutex;
use std::time::Duration;

// Use tokio's Instant so `tokio::time::pause()` can drive the limiter; tests do not have to
// actually wait.
use tokio::time::Instant;

pub struct RateLimiter {
    state: Mutex<State>,
    rate: f64,
    burst: f64,
}

struct State {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(qps: f32, burst: u32) -> Self {
        let rate = (qps as f64).max(0.1);
        let burst = (burst.max(1)) as f64;
        Self {
            state: Mutex::new(State {
                tokens: burst,
                last: Instant::now(),
            }),
            rate,
            burst,
        }
    }

    /// Take one token; sleep until one is available if there is none.
    pub async fn acquire(&self) {
        loop {
            let wait_secs = {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                st.tokens = (st.tokens
                    + now.saturating_duration_since(st.last).as_secs_f64() * self.rate)
                    .min(self.burst);
                st.last = now;
                if st.tokens >= 1.0 {
                    st.tokens -= 1.0;
                    return;
                }
                (1.0 - st.tokens) / self.rate
            };
            tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn burst_is_immediate_then_paced() {
        let limiter = RateLimiter::new(2.0, 2);
        let start = Instant::now();
        // The first two come out of the burst and do not wait
        limiter.acquire().await;
        limiter.acquire().await;
        assert!(start.elapsed() < Duration::from_millis(50));

        // The third must wait about 500 ms at 2 qps
        limiter.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(450));
    }
}
