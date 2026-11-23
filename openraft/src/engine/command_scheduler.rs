use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::config::Config;
use crate::engine::Command;
use crate::engine::engine_output::EngineOutput;
use crate::storage::RaftStateMachine;

/// Scheduler for reorganizing commands to improve I/O performance.
///
/// This scheduler optimizes the command queue by batching related operations together,
/// reducing the number of I/O operations and improving throughput.
pub(crate) struct CommandScheduler<'a, C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    config: &'a Config,
    output: &'a mut EngineOutput<C, SM>,
}

impl<'a, C, SM> CommandScheduler<'a, C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(config: &'a Config, output: &'a mut EngineOutput<C, SM>) -> Self {
        Self { config, output }
    }

    /// Merge consecutive `AppendEntries` commands at the front of the queue.
    ///
    /// This reduces storage I/O by combining multiple small writes into a single larger write.
    /// Only merges commands with the same `committed_vote` (i.e., from the same leader term).
    /// Stops when:
    /// - A non-AppendEntries command is encountered
    /// - A different leader's command is encountered
    /// - Merging the next command would bring the total entry count above
    ///   `config.max_append_entries`
    ///
    /// The entry limit prevents creating excessively large batches that could cause
    /// high latency in storage operations. It limits only what merging adds: the first command is
    /// kept whole, even when it already holds more entries than the limit.
    pub(crate) fn merge_front_append_entries(&mut self) {
        let max_entries = self.config.max_append_entries();
        let commands = &mut self.output.commands;

        let Some(c) = commands.pop_front() else {
            return;
        };

        let Command::AppendEntries {
            committed_vote,
            mut entries,
        } = c
        else {
            commands.push_front(c);
            return;
        };

        let mut n = 0;

        while let Some(next) = commands.pop_front() {
            match next {
                Command::AppendEntries {
                    committed_vote: next_vote,
                    entries: next_entries,
                } if next_vote == committed_vote && entries.len() as u64 + next_entries.len() as u64 <= max_entries => {
                    n += 1;
                    entries.extend(next_entries);
                }
                _ => {
                    commands.push_front(next);
                    break;
                }
            }
        }

        tracing::debug!(
            "CommandScheduler: merged {} AppendEntries, entries size: {}",
            n,
            entries.len()
        );

        commands.push_front(Command::AppendEntries {
            committed_vote,
            entries,
        });
    }
}

#[cfg(test)]
mod command_scheduler_test;
