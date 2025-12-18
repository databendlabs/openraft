use crate::RaftTypeConfig;
use crate::engine::Command;
use crate::engine::engine_output::EngineOutput;

/// Scheduler for reorganizing commands to improve I/O performance.
///
/// This scheduler optimizes the command queue by batching related operations together,
/// reducing the number of I/O operations and improving throughput.
pub(crate) struct CommandScheduler<'a, C>
where C: RaftTypeConfig
{
    output: &'a mut EngineOutput<C>,
}

impl<'a, C> CommandScheduler<'a, C>
where C: RaftTypeConfig
{
    pub(crate) fn new(output: &'a mut EngineOutput<C>) -> Self {
        Self { output }
    }

    /// Merge consecutive `AppendEntries` commands at the front of the queue.
    ///
    /// This reduces storage I/O by combining multiple small writes into a single larger write.
    /// Only merges commands with the same `committed_vote` (i.e., from the same leader term).
    /// Stops immediately when encountering a non-AppendEntries command or a different leader,
    /// preserving command ordering semantics.
    pub(crate) fn merge_front_append_entries(&mut self) {
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
                } if next_vote == committed_vote => {
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
mod tests {
    use super::CommandScheduler;
    use crate::engine::Command;
    use crate::engine::engine_output::EngineOutput;
    use crate::engine::testing::UTConfig;
    use crate::impls::Vote;
    use crate::testing::blank_ent;
    use crate::vote::raft_vote::RaftVoteExt;

    type C = UTConfig;

    fn committed_vote(term: u64, node_id: u64) -> crate::vote::committed::CommittedVote<C> {
        Vote::<C>::new(term, node_id).into_committed()
    }

    #[test]
    fn test_merge_empty_queue() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();
        assert!(output.commands.is_empty());
    }

    #[test]
    fn test_merge_single_command() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 1)],
        });
        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();
        assert_eq!(output.commands.len(), 1);
    }

    #[test]
    fn test_merge_front_not_append_entries() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        output.push_command(Command::SaveVote { vote: Vote::new(1, 0) });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 1)],
        });
        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();
        assert_eq!(output.commands.len(), 2);
    }

    #[test]
    fn test_merge_consecutive_same_vote() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 1)],
        });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 2)],
        });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 3)],
        });

        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();

        assert_eq!(output.commands.len(), 1);
        if let Command::AppendEntries { entries, .. } = &output.commands[0] {
            assert_eq!(entries.len(), 3);
        } else {
            panic!("Expected AppendEntries");
        }
    }

    #[test]
    fn test_merge_stops_at_different_vote() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 1)],
        });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(2, 0), // Different vote
            entries: vec![blank_ent(2, 0, 2)],
        });

        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();

        assert_eq!(output.commands.len(), 2);
    }

    #[test]
    fn test_merge_stops_at_non_append_entries() {
        let mut output: EngineOutput<C> = EngineOutput::new(8);
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 1)],
        });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 2)],
        });
        output.push_command(Command::SaveVote { vote: Vote::new(1, 0) });
        output.push_command(Command::AppendEntries {
            committed_vote: committed_vote(1, 0),
            entries: vec![blank_ent(1, 0, 3)],
        });

        let mut scheduler = CommandScheduler::new(&mut output);
        scheduler.merge_front_append_entries();

        // First two AppendEntries merged, SaveVote and last AppendEntries remain
        assert_eq!(output.commands.len(), 3);
        if let Command::AppendEntries { entries, .. } = &output.commands[0] {
            assert_eq!(entries.len(), 2);
        } else {
            panic!("Expected AppendEntries");
        }
    }
}
