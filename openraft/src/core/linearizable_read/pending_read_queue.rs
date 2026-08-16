use std::collections::BTreeMap;

use rt::OneshotSender;

use super::pending_read::PendingRead;
use super::pending_read_deadline_key::PendingReadDeadlineKey;
use super::pending_read_key::PendingReadKey;
use crate::RaftTypeConfig;
use crate::errors::LinearizableReadError;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

pub(crate) struct PendingReadQueue<C>
where C: RaftTypeConfig
{
    next_sequence: u64,

    /// The pending reads, ordered by the quorum-acknowledgement instant each one requires.
    reads: BTreeMap<PendingReadKey<InstantOf<C>>, PendingRead<C>>,

    /// A secondary index over the same reads, ordered by deadline.
    ///
    /// Each entry points at the primary key of the read it describes. A per-request
    /// `wait_timeout` makes deadline order differ from `reads` order, so expiry needs its own
    /// ordering to answer the earliest deadline first.
    deadlines: BTreeMap<PendingReadDeadlineKey<InstantOf<C>>, PendingReadKey<InstantOf<C>>>,
}

impl<C> Default for PendingReadQueue<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            next_sequence: 0,
            reads: BTreeMap::new(),
            deadlines: BTreeMap::new(),
        }
    }
}

impl<C> PendingReadQueue<C>
where C: RaftTypeConfig
{
    pub(crate) fn push(&mut self, min_quorum_acked_at: InstantOf<C>, pending_read: PendingRead<C>) {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.checked_add(1).expect("pending read sequence overflow");

        let key = PendingReadKey {
            min_quorum_acked_at,
            sequence,
        };
        let deadline_key = PendingReadDeadlineKey {
            deadline: pending_read.deadline,
            sequence,
        };
        self.reads.insert(key, pending_read);
        self.deadlines.insert(deadline_key, key);
        self.debug_assert_indexes_agree();
    }

    pub(crate) fn drain_expired(
        &mut self,
        now: InstantOf<C>,
        mut make_error: impl FnMut(InstantOf<C>) -> LinearizableReadError<C>,
    ) {
        while let Some((deadline_key, key)) = self.deadlines.first_key_value() {
            if deadline_key.deadline > now {
                break;
            }

            let key = *key;
            self.deadlines.pop_first();
            let pending_read = self.reads.remove(&key).expect("a deadline entry must have its read");
            let err = make_error(key.min_quorum_acked_at);
            pending_read.response_tx.send(Err(err)).ok();
        }
        self.debug_assert_indexes_agree();
    }

    /// Answer every read whose threshold is exceeded, reporting `applied` as the log id applied by
    /// the state machine now rather than when each read was queued.
    pub(crate) fn drain_satisfied(&mut self, quorum_acked_at: InstantOf<C>, applied: Option<LogIdOf<C>>) {
        while let Some((key, _)) = self.reads.first_key_value() {
            if key.min_quorum_acked_at >= quorum_acked_at {
                break;
            }

            let sequence = key.sequence;
            let (_, pending_read) = self.reads.pop_first().unwrap();
            let deadline_key = PendingReadDeadlineKey {
                deadline: pending_read.deadline,
                sequence,
            };
            self.deadlines.remove(&deadline_key).expect("a read must have its deadline entry");

            let linearizer = pending_read.linearizer.with_applied(applied.clone());
            pending_read.response_tx.send(Ok(linearizer)).ok();
        }
        self.debug_assert_indexes_agree();
    }

    pub(crate) fn drain_with_error(&mut self, err: LinearizableReadError<C>) {
        self.deadlines.clear();
        let pending_reads = std::mem::take(&mut self.reads);
        for pending_read in pending_reads.into_values() {
            pending_read.response_tx.send(Err(err.clone())).ok();
        }
    }

    pub(crate) fn earliest_deadline(&self) -> Option<InstantOf<C>> {
        let (deadline_key, _) = self.deadlines.first_key_value()?;
        Some(deadline_key.deadline)
    }

    /// Both indexes describe the same set of reads, so their sizes must match.
    fn debug_assert_indexes_agree(&self) {
        debug_assert_eq!(
            self.reads.len(),
            self.deadlines.len(),
            "pending read indexes hold different numbers of reads"
        );
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.debug_assert_indexes_agree();
        self.reads.is_empty()
    }
}
