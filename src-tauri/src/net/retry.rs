use std::time::Duration;

/// Exponential backoff: base * 2^attempt, capped at `max_ms`.
pub fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    let factor = 1u64.checked_shl(attempt.min(32)).unwrap_or(u64::MAX);
    Duration::from_millis(base_ms.saturating_mul(factor).min(max_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_and_caps() {
        assert_eq!(backoff(0, 500, 30_000), Duration::from_millis(500));
        assert_eq!(backoff(3, 500, 30_000), Duration::from_millis(4_000));
        assert_eq!(backoff(20, 500, 30_000), Duration::from_millis(30_000));
    }
}
