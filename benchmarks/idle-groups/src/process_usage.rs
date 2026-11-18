//! CPU time and memory of this process.

/// The user plus system CPU time of this process, in seconds, or `None` where `getrusage()` is
/// not available.
pub(crate) fn cpu_secs() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `getrusage()` fills `usage` when it returns 0.
        let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if ret != 0 {
            return None;
        }
        // SAFETY: `getrusage()` returned 0.
        let usage = unsafe { usage.assume_init() };
        let secs = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        Some(secs(usage.ru_utime) + secs(usage.ru_stime))
    }

    #[cfg(not(unix))]
    {
        None
    }
}

/// The memory this process holds, in KiB, wherever the OS keeps it, or `None` on other platforms.
///
/// - macOS: the physical footprint, as Activity Monitor shows it. RSS leaves out the pages that
///   macOS compresses, and macOS compresses the pages that idle groups leave untouched.
/// - Linux: `VmRSS` plus `VmSwap` from `/proc/self/status`.
pub(crate) fn memory_kib() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        footprint_kib()
    }

    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        resident_and_swapped_kib(&status)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// The physical footprint of this process, in KiB.
#[cfg(target_os = "macos")]
fn footprint_kib() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::uninit();
    let pid = std::process::id() as libc::c_int;
    // SAFETY: `proc_pid_rusage()` fills a `rusage_info_v2` for `RUSAGE_INFO_V2` when it returns 0.
    let ret = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast()) };
    if ret != 0 {
        return None;
    }
    // SAFETY: `proc_pid_rusage()` returned 0.
    let info = unsafe { info.assume_init() };
    Some(info.ri_phys_footprint / 1024)
}

/// The sum of the `VmRSS` and `VmSwap` fields of a Linux `/proc/<pid>/status`, in KiB.
///
/// Returns `None` without a `VmRSS` field. A missing `VmSwap` counts as 0.
#[cfg(any(target_os = "linux", test))]
fn resident_and_swapped_kib(status: &str) -> Option<u64> {
    let field_kib = |name: &str| {
        status.lines().find_map(|line| {
            let value = line.strip_prefix(name)?.strip_prefix(':')?;
            value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()
        })
    };
    Some(field_kib("VmRSS")? + field_kib("VmSwap").unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::cpu_secs;
    use super::memory_kib;
    use super::resident_and_swapped_kib;

    #[test]
    fn test_resident_and_swapped_kib_adds_swap_to_rss() {
        let status =
            "Name:\tidle-groups\nVmPeak:\t   9000 kB\nVmRSS:\t   1200 kB\nRssAnon:\t    800 kB\nVmSwap:\t     34 kB\n";
        assert_eq!(Some(1234), resident_and_swapped_kib(status));
    }

    #[test]
    fn test_resident_and_swapped_kib_requires_rss() {
        assert_eq!(Some(1200), resident_and_swapped_kib("VmRSS:\t   1200 kB\n"));
        assert_eq!(None, resident_and_swapped_kib("VmSwap:\t     34 kB\n"));
    }

    #[test]
    fn test_reads_the_usage_of_this_process() {
        if cfg!(unix) {
            assert!(cpu_secs().unwrap() >= 0.0);
        }
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(memory_kib().unwrap() > 0);
        }
    }
}
