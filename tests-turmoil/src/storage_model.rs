//! How the simulated disks keep what openraft writes to them.

use openraft::macros::since;
use serde::Serialize;

/// The storage faults one fuzz iteration runs with.
///
/// [`StorageModel::baseline`] is the model this harness had before these
/// switches existed: a write is on disk when its call returns, every append
/// reports its flush before `append` returns, and nothing is ever lost. Each
/// switch below departs from it only as far as openraft's storage contract
/// allows (`RaftLogStorage` and `IOFlushed` docs, and the `log-rocks` and
/// `log-wal` example stores):
///
/// - Log writes reach the disk in the order they were submitted. A crash keeps a prefix of them,
///   never a later write without an earlier one.
/// - `append` returns once its entries are readable; its callback fires once they are durable, and
///   callbacks may fire in any order.
/// - `save_vote` returns only once the vote, and so every write before it, is durable.
/// - `truncate_after`, `purge` and `save_committed` need no flush: a crash may undo them.
/// - The state machine is durable when `apply` returns, like `sm-rocks`.
#[since(
    version = "0.10.0",
    change = "Remove durable_append; allow callback jitter in both flush modes"
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorageModel {
    /// Flush log writes from a background task, a flush latency after they
    /// are submitted, instead of before the call returns. Until then a write
    /// is readable but not durable.
    pub async_flush: bool,

    /// Bounds, inclusive, of one flush's latency in ms (`async_flush` only).
    pub flush_delay_ms: (u64, u64),

    /// The chance that a flush stalls instead, for up to `flush_stall_ms`
    /// (`async_flush` only): the long tail of `fsync` latency.
    pub flush_stall_chance: f64,

    /// Upper bound, in ms, of a stalled flush.
    pub flush_stall_ms: u64,

    /// Deliver each flushed append's callback after its own random delay of
    /// up to this many ms, so callbacks reach openraft out of submission
    /// order. Applies to both flush modes. 0 delivers them at once, in order.
    pub callback_jitter_ms: u64,

    /// The chance that a crash drops every write not yet flushed
    /// (`async_flush` only): a power loss, or any crash of a store that
    /// buffers writes in the process, as `log-wal` does. Any other crash
    /// kills only the process, and the page cache still reaches the disk.
    pub power_loss_chance: f64,

    /// Upper bound, in ms, of a random delay before each `apply` batch.
    /// 0 applies at once.
    pub apply_delay_ms: u64,
}

impl StorageModel {
    /// Synchronous, lossless storage: the harness before these switches.
    #[since(version = "0.10.0", change = "Rename durable() to baseline()")]
    pub fn baseline() -> Self {
        Self {
            async_flush: false,
            flush_delay_ms: (0, 0),
            flush_stall_chance: 0.0,
            flush_stall_ms: 0,
            callback_jitter_ms: 0,
            power_loss_chance: 0.0,
            apply_delay_ms: 0,
        }
    }

    /// Whether every switch matches [`Self::baseline`].
    #[since(version = "0.10.0", change = "Rename is_durable() to is_baseline()")]
    pub fn is_baseline(&self) -> bool {
        *self == Self::baseline()
    }

    pub(crate) fn validate(&self) {
        let (min, max) = self.flush_delay_ms;
        assert!(min <= max, "flush_delay_ms bounds must be ordered");
        let valid_stall_chance = (0.0..=1.0).contains(&self.flush_stall_chance);
        assert!(valid_stall_chance, "flush_stall_chance must be within [0, 1]");
        let valid_power_loss_chance = (0.0..=1.0).contains(&self.power_loss_chance);
        assert!(valid_power_loss_chance, "power_loss_chance must be within [0, 1]");
        if self.flush_stall_chance > 0.0 {
            assert!(self.flush_stall_ms >= max, "flush_stall_ms must cover flush_delay_ms");
        }
    }
}

impl std::fmt::Display for StorageModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_baseline() {
            return write!(f, "durable");
        }
        if self.async_flush {
            write!(f, "flush {}..={}ms", self.flush_delay_ms.0, self.flush_delay_ms.1)?;
            if self.flush_stall_chance > 0.0 {
                write!(
                    f,
                    " (stall ≤{}ms {:.0}%)",
                    self.flush_stall_ms,
                    self.flush_stall_chance * 100.0
                )?;
            }
        } else {
            write!(f, "sync flush")?;
        }
        write!(
            f,
            ", callback jitter ≤{}ms, power loss {:.0}%",
            self.callback_jitter_ms,
            self.power_loss_chance * 100.0
        )?;
        if self.apply_delay_ms > 0 {
            write!(f, ", apply delay ≤{}ms", self.apply_delay_ms)?;
        }
        Ok(())
    }
}
