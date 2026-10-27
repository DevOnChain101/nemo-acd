use backoff::backoff_ms;

#[test]
fn doubles_each_attempt() {
    assert_eq!(backoff_ms(0, 100, 10_000), 100);
    assert_eq!(backoff_ms(1, 100, 10_000), 200);
    assert_eq!(backoff_ms(3, 100, 10_000), 800);
}

#[test]
fn caps_small_attempts() {
    assert_eq!(backoff_ms(10, 100, 10_000), 10_000);
}
