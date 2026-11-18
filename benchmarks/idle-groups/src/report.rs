use std::fmt;

use crate::mode::Mode;

/// What one measurement found.
#[derive(Debug, Clone)]
pub(crate) struct Report {
    pub(crate) groups: u64,
    pub(crate) mode: Mode,

    /// The tasks alive in the tokio runtime at the end of the measurement.
    pub(crate) tasks: usize,

    /// The memory of the process at the end of the measurement, in KiB, as
    /// [`memory_kib()`](crate::process_usage::memory_kib) reads it.
    pub(crate) memory_kib: Option<u64>,

    /// The CPU time of the process during the measurement, as a percentage of one core.
    pub(crate) cpu_percent: Option<f64>,

    /// The AppendEntries requests the stub followers received per second.
    pub(crate) appends_per_sec: f64,
}

impl fmt::Display for Report {
    /// Write one line of tab-separated `name=value` fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let per_group = |value: f64| value / self.groups as f64;

        write!(f, "groups={}\tmode={}", self.groups, self.mode)?;
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
        match self.cpu_percent {
            Some(cpu) => write!(f, "\tcpu%={cpu:.1}")?,
            None => write!(f, "\tcpu%=n/a")?,
        }
        write!(f, "\tappends/s={:.0}", self.appends_per_sec)
    }
}
