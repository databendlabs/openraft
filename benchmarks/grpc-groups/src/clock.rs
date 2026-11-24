/// Return the time of `CLOCK_MONOTONIC`, in nanoseconds.
///
/// Every process on a host reads the same `CLOCK_MONOTONIC`, so the driver and the nodes can
/// compare the times they take, which an `Instant` of one process does not allow.
pub(crate) fn monotonic_ns() -> u64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid `timespec` for `clock_gettime()` to write.
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    assert_eq!(0, ret, "CLOCK_MONOTONIC is readable");

    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

#[cfg(test)]
mod tests {
    use super::monotonic_ns;

    #[test]
    fn test_monotonic_ns_does_not_go_back() {
        let first = monotonic_ns();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = monotonic_ns();

        assert!(second >= first + 2_000_000, "first: {first}, second: {second}");
    }
}
