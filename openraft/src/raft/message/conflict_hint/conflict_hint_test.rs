use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::ConflictHint;

#[test]
fn shared_leader_with_different_end_indices_guides_probe() {
    let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 10), log_id(1, 1, 50), log_id(2, 1, 80)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    let expected = Some(log_id(1, 1, 40));
    assert_eq!(expected, common);
    assert_eq!(41, end);
}

#[test]
fn matching_tail_guides_probe_after_shorter_follower() {
    let local = LogIdList::new(None, [log_id(1, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 50)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    let expected = Some(log_id(1, 1, 50));
    assert_eq!(expected, common);
    assert_eq!(51, end);
}

#[test]
fn purged_ids_do_not_prove_a_mismatch_or_a_usable_prefix() {
    let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(2, 1, 80)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    assert_eq!(None, common);
    assert_eq!(80, end);
}

#[test]
fn empty_follower_starts_at_zero() {
    let local = LogIdList::new(None, [log_id(1, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    assert_eq!(None, common);
    assert_eq!(0, end);
}

#[test]
fn different_nodes_in_one_term_are_different_leaders() {
    let local = LogIdList::new(None, [log_id(3, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(3, 2, 80)]);
    let common = hint.last_common_log_id(&local);
    assert_eq!(None, common);
}

#[test]
fn first_matching_sample_guides_probe_without_matching_suffix_samples() {
    let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 30), log_id(2, 1, 80)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    let expected = Some(log_id(1, 1, 30));
    assert_eq!(expected, common);
    assert_eq!(80, end);
}

#[test]
fn purged_first_sample_is_not_assumed_matching() {
    let local = LogIdList::new(Some(log_id(2, 1, 40)), [log_id(4, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(3, 1, 80)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    assert_eq!(None, common);
    assert_eq!(80, end);
}

#[test]
fn shared_snapshot_boundary_allows_log_replication() {
    let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 50)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    let expected = Some(log_id(1, 1, 40));
    assert_eq!(expected, common);
    assert_eq!(41, end);
}

#[test]
fn an_old_sample_does_not_exclude_an_omitted_shared_leader() {
    let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 80), log_id(5, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 30), log_id(4, 1, 90)]);
    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);
    let expected = Some(log_id(1, 1, 30));
    assert_eq!(expected, common);
    assert_eq!(90, end);
}

#[test]
fn a_purge_boundary_still_proves_a_mismatch() {
    let local = LogIdList::new(Some(log_id(2, 1, 40)), [log_id(4, 1, 100)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 40), log_id(3, 1, 80)]);

    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);

    assert_eq!(None, common);
    assert_eq!(40, end);
}

#[test]
fn a_sample_beyond_the_local_log_adds_no_bound() {
    let local = LogIdList::new(None, [log_id(1, 1, 10)]);
    let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 5), log_id(2, 1, 20)]);

    let common = hint.last_common_log_id(&local);
    let end = hint.common_prefix_upper_bound(&local);

    let expected = Some(log_id(1, 1, 5));
    assert_eq!(expected, common);
    assert_eq!(21, end);
}
