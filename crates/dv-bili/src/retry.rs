//! Retry policy: exponential backoff plus jitter, with a longer wait for anti-bot protection
//! (-412).

use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dv_core::{Result, VaultError};

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n;
        self
    }

    /// Run `op`; retryable errors are retried with backoff.
    pub async fn run<T, F, Fut>(&self, mut op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut attempt: u32 = 0;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if !err.is_retryable() || attempt >= self.max_attempts {
                        return Err(err);
                    }
                    attempt += 1;
                    let delay = self.delay_for(attempt, &err);
                    tracing::warn!(attempt, delay_ms = delay.as_millis(), error = %err, "request failed, backing off before retrying");
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    fn delay_for(&self, attempt: u32, err: &VaultError) -> Duration {
        // Anti-bot protection needs at least a minute; retrying harder only makes it worse.
        let rate_limited = matches!(err, VaultError::RateLimited);
        let base = if rate_limited {
            Duration::from_secs(60)
        } else {
            self.base_delay
        };
        let cap = if rate_limited {
            Duration::from_secs(300)
        } else {
            self.max_delay
        };
        let factor = 1u32 << (attempt - 1).min(6);
        let backoff = (base * factor).min(cap);
        backoff + jitter(backoff)
    }
}

/// ±20% jitter so a batch of requests does not march in lockstep.
fn jitter(base: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let ratio = (nanos % 401) as f64 / 1000.0 - 0.2; // -0.2 ..= 0.2
    base.mul_f64(ratio.abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test(start_paused = true)]
    async fn retries_then_succeeds() {
        let calls = Arc::new(AtomicU32::new(0));
        let policy = RetryPolicy::default().with_max_attempts(3);

        let c = calls.clone();
        let out: Result<u8> = policy
            .run(|| {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        Err(VaultError::Network("boom".into()))
                    } else {
                        Ok(7)
                    }
                }
            })
            .await;

        assert_eq!(out.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_retry_permanent_errors() {
        let calls = Arc::new(AtomicU32::new(0));
        let policy = RetryPolicy::default().with_max_attempts(5);
        let c = calls.clone();
        let out: Result<()> = policy
            .run(|| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(VaultError::Api {
                        code: -404,
                        msg: "gone".into(),
                    })
                }
            })
            .await;

        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
