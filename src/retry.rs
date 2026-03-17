//! # Retry Policies
//!
//! Configurable retry logic for Cortex API operations, with exponential
//! backoff and error-category awareness.
//!
//! Non-retryable errors (e.g. [`CortexError::NoHeadsetFound`], [`CortexError::NotApproved`])
//! are returned immediately regardless of the policy. Only errors where
//! [`CortexError::is_retryable`] returns `true` trigger a retry.
//!
//! ## Predefined Policies
//!
//! | Policy | Max Retries | Use Case |
//! |--------|-------------|----------|
//! | [`RetryPolicy::query()`] | 3 | Idempotent reads: `getCortexInfo`, `queryHeadsets` |
//! | [`RetryPolicy::idempotent()`] | 2 | Safe state changes: `subscribe`, `controlDevice` |
//! | [`RetryPolicy::none()`] | 0 | Non-idempotent: `authorize`, `createSession`, `injectMarker` |
//!
//! ## Example
//!
//! ```rust
//! use emotiv::retry::{RetryPolicy, with_retry};
//! use emotiv::error::CortexError;
//! use std::sync::atomic::{AtomicUsize, Ordering};
//!
//! let attempts = AtomicUsize::new(0);
//! let rt = tokio::runtime::Builder::new_current_thread()
//!     .enable_time()
//!     .build()
//!     .unwrap();
//!
//! let result = rt.block_on(async {
//!     with_retry(&RetryPolicy::query(), || {
//!         let attempt = attempts.fetch_add(1, Ordering::SeqCst);
//!         async move {
//!             if attempt == 0 {
//!                 Err(CortexError::Timeout { seconds: 1 })
//!             } else {
//!                 Ok::<_, CortexError>(42)
//!             }
//!         }
//!     })
//!     .await
//! });
//!
//! assert_eq!(result.unwrap(), 42);
//! assert_eq!(attempts.load(Ordering::SeqCst), 2);
//! ```

use std::time::Duration;

use crate::error::{CortexError, CortexResult};

/// Policy controlling how failed Cortex API operations are retried.
#[derive(Debug, Clone)]
pub enum RetryPolicy {
    /// No retries — fail immediately on error.
    ///
    /// Use for non-idempotent operations that must not be repeated:
    /// `authorize`, `createSession`, `createRecord`, `injectMarker`.
    None,

    /// Retry with exponential backoff.
    Backoff {
        /// Maximum number of retry attempts (not counting the initial attempt).
        max_retries: u32,

        /// Initial delay before the first retry.
        base_delay: Duration,

        /// Maximum delay between retries (exponential backoff cap).
        max_delay: Duration,
    },
}

impl RetryPolicy {
    /// No retries. Use for non-idempotent operations.
    #[must_use]
    pub fn none() -> Self {
        Self::None
    }

    /// 3 retries, 500 ms base delay, 10 s max.
    ///
    /// Use for idempotent query operations: `getCortexInfo`, `queryHeadsets`,
    /// `querySessions`, `queryRecords`.
    #[must_use]
    pub fn query() -> Self {
        Self::Backoff {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }

    /// 2 retries, 1 s base delay, 15 s max.
    ///
    /// Use for idempotent state-changing operations: `controlDevice`,
    /// `subscribe`, `unsubscribe`, `setupProfile(load)`.
    #[must_use]
    pub fn idempotent() -> Self {
        Self::Backoff {
            max_retries: 2,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(15),
        }
    }

    /// 2 retries, 1 s base delay, 15 s max.
    ///
    /// Use for idempotent stopping operations: `updateSession(close)`,
    /// `updateRecord(stop)`.
    #[must_use]
    pub fn stop() -> Self {
        Self::Backoff {
            max_retries: 2,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(15),
        }
    }

    /// Custom backoff policy.
    #[must_use]
    pub fn custom(max_retries: u32, base_delay: Duration, max_delay: Duration) -> Self {
        Self::Backoff {
            max_retries,
            base_delay,
            max_delay,
        }
    }
}

/// Execute an async operation according to the given retry policy.
///
/// The operation is retried only when [`CortexError::is_retryable()`] returns
/// `true`. Non-retryable errors are returned immediately regardless of the policy.
///
/// On exhaustion, returns [`CortexError::RetriesExhausted`] wrapping
/// the last error encountered.
///
/// # Errors
///
/// Returns any error produced by `operation`, or [`CortexError::RetriesExhausted`]
/// when all retry attempts are used up.
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, mut operation: F) -> CortexResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = CortexResult<T>>,
{
    match policy {
        RetryPolicy::None => operation().await,
        RetryPolicy::Backoff {
            max_retries,
            base_delay,
            max_delay,
        } => {
            let mut delay = *base_delay;

            for attempt in 0..=*max_retries {
                match operation().await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        // Non-retryable errors fail immediately
                        if !e.is_retryable() {
                            return Err(e);
                        }

                        // Last attempt — wrap in RetriesExhausted
                        if attempt == *max_retries {
                            return Err(CortexError::RetriesExhausted {
                                attempts: attempt + 1,
                                last_error: Box::new(e),
                            });
                        }

                        tracing::warn!(
                            attempt = attempt + 1,
                            max = max_retries + 1,
                            error = %e,
                            delay_ms = delay.as_millis() as u64,
                            "Retrying after transient error"
                        );

                        tokio::time::sleep(delay).await;

                        // Exponential backoff with cap
                        delay = std::cmp::min(delay * 2, *max_delay);
                    }
                }
            }

            // Unreachable — but handle gracefully rather than panic
            operation().await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn test_policy_none_returns_immediately() {
        let result: CortexResult<i32> = with_retry(&RetryPolicy::none(), || async {
            Err(CortexError::Timeout { seconds: 1 })
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_second_attempt() {
        let attempts = AtomicUsize::new(0);
        let result = with_retry(&RetryPolicy::custom(3, Duration::from_millis(1), Duration::from_millis(10)), || {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(CortexError::Timeout { seconds: 1 })
                } else {
                    Ok::<i32, CortexError>(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_non_retryable_fails_immediately() {
        let attempts = AtomicUsize::new(0);
        let result: CortexResult<i32> = with_retry(&RetryPolicy::query(), || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(CortexError::NoHeadsetFound) }
        })
        .await;

        assert!(matches!(result, Err(CortexError::NoHeadsetFound)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1); // not retried
    }

    #[tokio::test]
    async fn test_retries_exhausted() {
        let attempts = AtomicUsize::new(0);
        let result: CortexResult<i32> = with_retry(
            &RetryPolicy::custom(2, Duration::from_millis(1), Duration::from_millis(10)),
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err(CortexError::Timeout { seconds: 1 }) }
            },
        )
        .await;

        assert!(matches!(result, Err(CortexError::RetriesExhausted { attempts: 3, .. })));
        assert_eq!(attempts.load(Ordering::SeqCst), 3); // initial + 2 retries
    }

    #[test]
    fn test_policy_constructors() {
        assert!(matches!(RetryPolicy::none(), RetryPolicy::None));
        assert!(matches!(RetryPolicy::query(), RetryPolicy::Backoff { max_retries: 3, .. }));
        assert!(matches!(RetryPolicy::idempotent(), RetryPolicy::Backoff { max_retries: 2, .. }));
    }
}
