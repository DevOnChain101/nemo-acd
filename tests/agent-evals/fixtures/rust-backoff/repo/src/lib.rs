/// Exponential retry delay: `base_ms * 2^attempt`, never more than `cap_ms`.
pub fn backoff_ms(attempt: u32, base_ms: u64, cap_ms: u64) -> u64 {
    let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    base_ms.saturating_mul(factor).min(cap_ms)
}
