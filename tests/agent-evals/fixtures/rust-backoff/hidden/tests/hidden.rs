use backoff::backoff_ms;

#[test]
fn huge_attempts_hit_the_cap() {
    assert_eq!(backoff_ms(63, 100, 30_000), 30_000);
    assert_eq!(backoff_ms(64, 100, 30_000), 30_000);
    assert_eq!(backoff_ms(1000, 1, 30_000), 30_000);
    assert_eq!(backoff_ms(u32::MAX, 100, 30_000), 30_000);
}

#[test]
fn mul_overflow_is_capped() {
    assert_eq!(backoff_ms(40, u64::MAX / 2, 5_000), 5_000);
}

#[test]
fn normal_values_unchanged() {
    assert_eq!(backoff_ms(0, 100, 30_000), 100);
    assert_eq!(backoff_ms(4, 100, 30_000), 1_600);
    assert_eq!(backoff_ms(9, 100, 30_000), 30_000);
    assert_eq!(backoff_ms(5, 100, 50), 50);
}
