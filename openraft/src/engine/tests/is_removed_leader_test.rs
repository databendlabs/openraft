use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::raft_msg::leader_removal::LeaderRemoval;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

/// Voters 1,2: node 1 is a member.
fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

/// Voters 2,3; node 1 is removed.
fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

/// Voters 2,3,4; node 1 is removed.
fn m234() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2,3,4}], [])
}

fn stored(log_index: u64, mem: Membership<u64, ()>) -> Arc<StoredMembershipOf<UTConfig>> {
    Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, log_index)), mem))
}

/// The removal of node 1 under the vote of `term`, by the membership config at `log_index`.
fn removal(term: u64, log_index: u64) -> LeaderRemoval<UTConfig> {
    LeaderRemoval {
        vote: Vote::new_committed(term, 1),
        membership_log_id: Some(log_id(2, 1, log_index)),
    }
}

/// A Leader of node 1 with the vote of term 3, then the effective and committed membership config
/// `m23()` at log index 3, which removes it.
fn eng_removed_leader() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));

    // Establish the Leader while it is a member, then remove it with a committed config.
    let member = stored(1, m12());
    eng.state.membership_state = MembershipState::new(member.clone(), member);
    eng.testing_new_leader();

    let removing = stored(3, m23());
    eng.state.membership_state = MembershipState::new(removing.clone(), removing);
    eng.output.clear_commands();

    eng
}

#[test]
fn test_is_removed_leader_matching_removal() {
    let eng = eng_removed_leader();

    assert!(eng.is_removed_leader(&removal(3, 3)));
}

#[test]
fn test_is_removed_leader_not_a_leader() {
    let mut eng = eng_removed_leader();
    eng.leader = None;

    assert!(!eng.is_removed_leader(&removal(3, 3)));
}

/// A removal observed under another vote is stale, even if the membership config is the same.
#[test]
fn test_is_removed_leader_another_vote() {
    let eng = eng_removed_leader();

    assert!(!eng.is_removed_leader(&removal(2, 3)));
}

/// A readmission appends a membership config that contains this node: the removal is stale before
/// the readmission commits.
#[test]
fn test_is_removed_leader_readmitted() {
    let mut eng = eng_removed_leader();
    let readmitting = Membership::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1});
    eng.state.membership_state.set_effective(stored(4, readmitting));

    assert!(!eng.is_removed_leader(&removal(3, 3)));
}

/// A newer committed membership config that removes this node too is another removal: only that
/// one holds.
#[test]
fn test_is_removed_leader_newer_committed_membership() {
    let mut eng = eng_removed_leader();
    let newer = stored(4, m234());
    eng.state.membership_state = MembershipState::new(newer.clone(), newer);

    assert!(!eng.is_removed_leader(&removal(3, 3)));
    assert!(eng.is_removed_leader(&removal(3, 4)));
}

/// The removal names the effective membership config, but that config is not committed.
#[test]
fn test_is_removed_leader_uncommitted_membership() {
    let mut eng = eng_removed_leader();
    eng.state.membership_state.set_effective(stored(4, m234()));

    assert!(!eng.is_removed_leader(&removal(3, 4)));
}

/// The removal names the effective and committed membership config, but that config contains this
/// node.
#[test]
fn test_is_removed_leader_membership_contains_this_node() {
    let mut eng = eng_removed_leader();
    let member = stored(4, Membership::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1}));
    eng.state.membership_state = MembershipState::new(member.clone(), member);

    assert!(!eng.is_removed_leader(&removal(3, 4)));
}
