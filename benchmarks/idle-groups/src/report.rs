use std::fmt;

use crate::followers::Followers;
use crate::mode::Mode;

/// What one measurement found.
#[derive(Debug, Clone)]
pub(crate) struct Report {
    pub(crate) groups: u64,
    pub(crate) followers: Followers,
    pub(crate) mode: Mode,

    /// The tasks alive in the tokio runtime at the end of the measurement.
    pub(crate) tasks: usize,

    /// The memory of the process at the end of the measurement, in KiB, as
    /// [`memory_kib()`](crate::process_usage::memory_kib) reads it.
    pub(crate) memory_kib: Option<u64>,

    /// The heap memory requested and not yet freed at the end of the measurement, in KiB, as
    /// [`CountingAlloc`](crate::counting_alloc::CountingAlloc) counts it, or `None` without
    /// `--count-allocs`.
    pub(crate) heap_kib: Option<u64>,

    /// The CPU time of the process during the measurement, as a percentage of one core.
    pub(crate) cpu_percent: Option<f64>,

    /// The heap allocations per second during the measurement, including reallocations, or `None`
    /// without `--count-allocs`.
    pub(crate) allocs_per_sec: Option<f64>,

    /// The AppendEntries requests the followers received per second.
    pub(crate) appends_per_sec: f64,

    /// The elections during the measurement: how far the highest term of each group grew, summed
    /// over the groups.
    pub(crate) elections: u64,

    /// The heartbeats, elections and pending-read checks whose deadline was reached, per second,
    /// or `None` without the `runtime-stats` feature.
    pub(crate) fires_per_sec: Option<f64>,

    /// The wakes that `RaftCore` gave a timer's task to re-arm it, per second, or `None` without
    /// the `runtime-stats` feature.
    pub(crate) rearms_per_sec: Option<f64>,

    /// The `RaftMetrics` sends per second, one per metrics report, or `None` without the
    /// `runtime-stats` feature.
    pub(crate) metrics_per_sec: Option<f64>,

    /// The `RaftDataMetrics` sends per second, made only when a field changed, or `None` without
    /// the `runtime-stats` feature.
    pub(crate) data_metrics_per_sec: Option<f64>,

    /// The `RaftServerMetrics` sends per second, made only when a field changed, or `None`
    /// without the `runtime-stats` feature.
    pub(crate) server_metrics_per_sec: Option<f64>,
}

impl fmt::Display for Report {
    /// Write one line of tab-separated `name=value` fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let per_group = |value: f64| value / self.groups as f64;

        write!(
            f,
            "groups={}\tfollowers={}\tmode={}",
            self.groups, self.followers, self.mode
        )?;
        write!(
            f,
            "\ttasks={}\ttasks/group={:.1}",
            self.tasks,
            per_group(self.tasks as f64)
        )?;
        match self.memory_kib {
            Some(kib) => write!(
                f,
                "\tmemory_mib={:.0}\tmemory_kib/group={:.0}",
                kib as f64 / 1024.0,
                per_group(kib as f64)
            )?,
            None => write!(f, "\tmemory_mib=n/a\tmemory_kib/group=n/a")?,
        }
        match self.heap_kib {
            Some(kib) => write!(
                f,
                "\theap_mib={:.0}\theap_kib/group={:.0}",
                kib as f64 / 1024.0,
                per_group(kib as f64)
            )?,
            None => write!(f, "\theap_mib=n/a\theap_kib/group=n/a")?,
        }
        match self.cpu_percent {
            Some(cpu) => write!(f, "\tcpu%={cpu:.1}")?,
            None => write!(f, "\tcpu%=n/a")?,
        }
        match self.allocs_per_sec {
            Some(allocs) => write!(f, "\tallocs/s={allocs:.0}")?,
            None => write!(f, "\tallocs/s=n/a")?,
        }
        write!(f, "\tappends/s={:.0}", self.appends_per_sec)?;
        write!(f, "\telections={}", self.elections)?;

        let rates = [
            ("fires/s", self.fires_per_sec),
            ("rearms/s", self.rearms_per_sec),
            ("metrics/s", self.metrics_per_sec),
            ("data_metrics/s", self.data_metrics_per_sec),
            ("server_metrics/s", self.server_metrics_per_sec),
        ];
        for (name, rate) in rates {
            match rate {
                Some(rate) => write!(f, "\t{name}={rate:.0}")?,
                None => write!(f, "\t{name}=n/a")?,
            }
        }
        Ok(())
    }
}
