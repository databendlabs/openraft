use futures_util::FutureExt;
use futures_util::Stream;
use futures_util::StreamExt;

use super::MAX_PREFETCH_ENTRIES;
use super::job::Job;
use crate::RaftTypeConfig;
use crate::core::sm::Command;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::storage::RaftStateMachine;

/// Admission metadata and the complete loaded prefix, separate from the owned I/O jobs.
pub(super) struct Pipeline<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pending: Option<ApplyBatch<C>>,
    ready: Option<ApplyBatch<C>>,
    held: Option<Command<C, SM>>,
    admitted_end: Option<u64>,
    reserved_entries: u64,
    barrier_running: bool,
    input_closed: bool,
}

impl<C, SM> Default for Pipeline<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn default() -> Self {
        Self {
            pending: None,
            ready: None,
            held: None,
            admitted_end: None,
            reserved_entries: 0,
            barrier_running: false,
            input_closed: false,
        }
    }
}

impl<C, SM> Pipeline<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(super) fn can_receive(&self) -> bool {
        !self.input_closed
            && !self.barrier_running
            && self.held.is_none()
            && self.reserved_entries < MAX_PREFETCH_ENTRIES
    }

    /// Polling `next` retains the receive future inside the pinned unfold stream.
    pub(super) fn collect_ready<S>(&mut self, input: &mut S)
    where S: Stream<Item = Command<C, SM>> + Unpin {
        while self.can_receive() {
            let next = input.next();
            let Some(command) = next.now_or_never() else { break };
            self.receive(command);
        }
    }

    pub(super) fn receive(&mut self, command: Option<Command<C, SM>>) {
        let Some(command) = command else {
            self.input_closed = true;
            return;
        };
        if self.fits(&command) {
            self.admit(command);
        } else {
            self.held = Some(command);
        }
    }

    fn fits(&self, command: &Command<C, SM>) -> bool {
        let Command::Apply { first, last, .. } = command else {
            return false;
        };
        let since = first.index();
        let last_index = last.index();
        let end = last_index + 1;
        let contiguous = self.admitted_end.is_none_or(|admitted_end| since == admitted_end);
        let entries = end - since;
        let remaining = MAX_PREFETCH_ENTRIES.saturating_sub(self.reserved_entries);
        contiguous && entries <= remaining
    }

    fn admit(&mut self, command: Command<C, SM>) {
        let Command::Apply {
            first,
            last,
            client_resp_channels,
        } = command
        else {
            unreachable!("only apply metadata is admitted");
        };
        let batch = ApplyBatch::new(first, last, client_resp_channels);
        let since = batch.since();
        let end = batch.end();
        let entries = end - since;
        self.reserved_entries += entries;
        self.admitted_end = Some(end);
        match &mut self.pending {
            Some(pending) => pending.extend(batch),
            None => self.pending = Some(batch),
        }
    }

    /// Coalesce ready input and completed reads before this policy chooses the next job.
    /// A loaded prefix starts before new I/O; completed reads were already coalesced.
    pub(super) fn schedule(&mut self, machine_idle: bool, reader_idle: bool) -> Option<Job<C, SM>> {
        if self.barrier_running {
            return None;
        }
        let foreground = machine_idle && reader_idle && self.reserved_entries == 0;
        let admit_held = match &self.held {
            Some(command) => foreground || self.fits(command),
            None => false,
        };
        if admit_held && let Some(command) = self.held.take() {
            if let Command::Apply { .. } = &command {
                self.admit(command);
            } else {
                self.barrier_running = true;
                self.admitted_end = None;
                return Some(Job::Command(command));
            }
        }
        if machine_idle && let Some(batch) = self.ready.take() {
            let since = batch.since();
            let end = batch.end();
            let entries = end - since;
            self.reserved_entries -= entries;
            return Some(Job::Apply(batch));
        }
        if reader_idle && let Some(batch) = self.pending.take() {
            return Some(Job::Read(batch));
        }
        None
    }

    pub(super) fn read_finished(&mut self, batch: ApplyBatch<C>) {
        match &mut self.ready {
            Some(ready) => ready.extend(batch),
            None => self.ready = Some(batch),
        }
    }

    pub(super) fn machine_finished(&mut self) {
        self.barrier_running = false;
    }

    pub(super) fn finished(&self, machine_idle: bool, reader_idle: bool) -> bool {
        self.input_closed && machine_idle && reader_idle && self.reserved_entries == 0 && self.held.is_none()
    }
}
