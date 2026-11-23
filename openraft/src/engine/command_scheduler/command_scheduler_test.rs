use super::CommandScheduler;
use crate::batch::Batch;
use crate::config::Config;
use crate::engine::Command;
use crate::engine::engine_output::EngineOutput;
use crate::engine::testing::UTConfig;
use crate::engine::testing::UTLeaderId;
use crate::entry::RaftEntry;
use crate::impls::Vote;
use crate::testing::log_id;
use crate::type_config::alias::EntryOf;
use crate::vote::committed::CommittedVote;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

fn committed_vote(term: u64, node_id: u64) -> CommittedVote<UTLeaderId> {
    Vote::<UTLeaderId>::new(term, node_id).to_committed()
}

/// A normal entry at `index` of the leader `(term, node_id)`, whose data is its own index times 10,
/// so that entries with equal log ids but other payloads, or entries out of order, differ.
fn ent(term: u64, node_id: u64, index: u64) -> EntryOf<C> {
    EntryOf::<C>::new_normal(log_id::<C>(term, node_id, index), index * 10)
}

/// An `AppendEntries` of the leader `(term, node_id)` with one entry per index in `indexes`.
fn append(term: u64, node_id: u64, indexes: impl IntoIterator<Item = u64>) -> Command<C> {
    Command::AppendEntries {
        committed_vote: committed_vote(term, node_id),
        entries: Batch::of(indexes.into_iter().map(|i| ent(term, node_id, i))),
    }
}

fn save_vote() -> Command<C> {
    Command::SaveVote { vote: Vote::new(1, 0) }
}

/// Merge the front of `commands` with `config`, and return the whole queue.
fn schedule(config: &Config, commands: Vec<Command<C>>) -> Vec<Command<C>> {
    let mut output: EngineOutput<C> = EngineOutput::default();
    for command in commands {
        output.push_command(command);
    }
    let mut scheduler = CommandScheduler::new(config, &mut output);
    scheduler.merge_front_append_entries();
    output.take_commands()
}

/// Drop the front command of `queue`, then schedule the rest again: the commands that a boundary
/// kept apart are still whole and in order.
fn schedule_after_front(config: &Config, queue: Vec<Command<C>>) -> Vec<Command<C>> {
    schedule(config, queue.into_iter().skip(1).collect())
}

fn config_with_max_append(max_append_entries: u64) -> Config {
    Config {
        max_append_entries: Some(max_append_entries),
        ..Default::default()
    }
}

#[test]
fn test_merge_empty_queue() {
    let got = schedule(&Config::default(), vec![]);
    assert!(got.is_empty());
}

#[test]
fn test_merge_single_command() {
    let got = schedule(&Config::default(), vec![append(1, 0, [1])]);
    assert_eq!(vec![append(1, 0, [1])], got);
}

#[test]
fn test_merge_front_not_append_entries() {
    let got = schedule(&Config::default(), vec![save_vote(), append(1, 0, [1])]);
    assert_eq!(vec![save_vote(), append(1, 0, [1])], got);
}

/// Same-vote appends merge into one, with every entry in its order.
#[test]
fn test_merge_consecutive_same_vote() {
    let got = schedule(&Config::default(), vec![
        append(1, 0, [1]),
        append(1, 0, [2]),
        append(1, 0, [3]),
    ]);
    assert_eq!(vec![append(1, 0, [1, 2, 3])], got);
}

#[test]
fn test_merge_stops_at_different_vote() {
    let got = schedule(&Config::default(), vec![append(1, 0, [1]), append(2, 0, [2])]);
    assert_eq!(vec![append(1, 0, [1]), append(2, 0, [2])], got);
}

/// Two leaders of one term have different votes, so their appends stay apart.
#[test]
fn test_merge_stops_at_another_leader_of_the_same_term() {
    let got = schedule(&Config::default(), vec![append(1, 0, [1]), append(1, 1, [2])]);
    assert_eq!(vec![append(1, 0, [1]), append(1, 1, [2])], got);
}

/// A command that is not an append ends the merge, and keeps its place before the appends after it.
#[test]
fn test_merge_stops_at_non_append_entries() {
    let config = Config::default();
    let got = schedule(&config, vec![
        append(1, 0, [1]),
        append(1, 0, [2]),
        save_vote(),
        append(1, 0, [3]),
    ]);
    assert_eq!(vec![append(1, 0, [1, 2]), save_vote(), append(1, 0, [3])], got);

    let got = schedule_after_front(&config, got);
    assert_eq!(vec![save_vote(), append(1, 0, [3])], got);

    let got = schedule_after_front(&config, got);
    assert_eq!(vec![append(1, 0, [3])], got);
}

/// A merge may fill the limit exactly; the append that would exceed it stays whole.
#[test]
fn test_merge_stops_at_max_entries() {
    let config = config_with_max_append(3);
    let got = schedule(&config, vec![
        append(1, 0, [1, 2]),
        append(1, 0, [3]),
        append(1, 0, [4]),
    ]);
    assert_eq!(vec![append(1, 0, [1, 2, 3]), append(1, 0, [4])], got);

    let got = schedule_after_front(&config, got);
    assert_eq!(vec![append(1, 0, [4])], got);
}

/// A limit of 0 merges no two appends that hold entries.
#[test]
fn test_merge_nothing_with_zero_max_entries() {
    let got = schedule(&config_with_max_append(0), vec![append(1, 0, [1]), append(1, 0, [2])]);
    assert_eq!(vec![append(1, 0, [1]), append(1, 0, [2])], got);
}

/// The first append stays whole even when it already holds more entries than the limit.
#[test]
fn test_merge_keeps_an_oversized_first_append_whole() {
    let got = schedule(&config_with_max_append(2), vec![
        append(1, 0, [1, 2, 3]),
        append(1, 0, [4]),
    ]);
    assert_eq!(vec![append(1, 0, [1, 2, 3]), append(1, 0, [4])], got);
}

/// An append that does not fit in the room the first one leaves stays whole, and nothing after it
/// is merged past it.
#[test]
fn test_merge_stops_at_an_append_that_does_not_fit() {
    let config = config_with_max_append(3);
    let got = schedule(&config, vec![
        append(1, 0, [1]),
        append(1, 0, [2, 3, 4]),
        append(1, 0, [5]),
    ]);
    assert_eq!(vec![append(1, 0, [1]), append(1, 0, [2, 3, 4]), append(1, 0, [5])], got);

    let got = schedule_after_front(&config, got);
    assert_eq!(vec![append(1, 0, [2, 3, 4]), append(1, 0, [5])], got);
}
