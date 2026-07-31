//! Exposes internal types and setup helpers for criterion benchmarks.
//!
//! This module is gated behind `#[cfg(feature = "bench")]` and is not part of the public API.

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryPayloadOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

pub type BenchEntryPayload = EntryPayloadOf<UTConfig>;

/// A wrapper around Engine that exposes only what benchmarks need.
pub struct BenchEngine {
    eng: Engine<UTConfig>,
    iter_count: u64,
}

impl BenchEngine {
    pub fn new_leader() -> Self {
        let mut eng = Engine::testing_default(0);
        eng.state.enable_validation(false);

        eng.config.id = 1;
        eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
        eng.state.vote = Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(3, 1),
        );
        eng.state.log_ids.append(log_id(1, 1, 1));
        eng.state.log_ids.append(log_id(2, 1, 3));
        eng.state.membership_state = MembershipState::new(
            Arc::new(StoredMembershipOf::<UTConfig>::new(
                Some(log_id(1, 1, 1)),
                Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0, 1}], []),
            )),
            Arc::new(StoredMembershipOf::<UTConfig>::new(
                Some(log_id(2, 1, 3)),
                Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2, 3}], btreeset! {1, 2, 3}),
            )),
        );
        eng.testing_new_leader();
        eng.state.server_state = eng.calc_server_state();
        eng.output.clear_commands();

        Self { eng, iter_count: 0 }
    }

    pub fn leader_append_entries(
        &mut self,
        payloads: impl IntoIterator<Item = EntryPayloadOf<UTConfig>> + AsRef<[EntryPayloadOf<UTConfig>]>,
    ) {
        self.eng.try_leader_handler().unwrap().leader_append_entries(payloads);

        self.iter_count += 1;
        if self.iter_count.is_multiple_of(64) {
            self.eng.output.clear_commands();
        }
    }
}

/// A wrapper around VecProgress for benchmarking.
pub struct BenchVecProgress {
    progress: VecProgress<(u64, u64), Vec<std::collections::BTreeSet<u64>>>,
    id: u64,
    values: [u64; 8],
}

impl BenchVecProgress {
    pub fn new_joint_01234_567() -> Self {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}, btreeset! {5, 6, 7}];
        let progress = VecProgress::<(u64, u64), _>::new(quorum_set, 0..=7, |id| (id, 0));
        Self {
            progress,
            id: 0,
            values: [0, 1, 2, 3, 4, 5, 6, 7],
        }
    }

    pub fn update_next(&mut self) {
        self.id = (self.id + 1) & 7;
        self.values[self.id as usize] += 1;
        let v = self.values[self.id as usize];
        self.progress.update(&self.id, v).ok();
    }
}

/// Quorum set benchmark: check if a slice of IDs forms a quorum of a slice quorum-set.
pub fn quorum_slice_is_quorum_slice(ids: &[usize], quorum_set: &[usize]) -> bool {
    quorum_set.is_quorum(ids.iter())
}

/// Quorum set benchmark: check if a slice of IDs forms a quorum of a BTreeSet quorum-set.
pub fn quorum_btreeset_is_quorum_slice(ids: &[usize], quorum_set: &std::collections::BTreeSet<usize>) -> bool {
    quorum_set.is_quorum(ids.iter())
}

/// Quorum set benchmark: check if a slice of IDs forms a quorum of a joint (Vec<BTreeSet>)
/// quorum-set.
pub fn quorum_joint_is_quorum_slice(ids: &[usize], quorum_set: &Vec<std::collections::BTreeSet<usize>>) -> bool {
    quorum_set.is_quorum(ids.iter())
}

/// Quorum set benchmark: check if a BTreeSet of IDs forms a quorum of a joint quorum-set.
pub fn quorum_joint_is_quorum_btreeset(
    ids: &std::collections::BTreeSet<usize>,
    quorum_set: &Vec<std::collections::BTreeSet<usize>>,
) -> bool {
    quorum_set.is_quorum(ids.iter())
}

/// Membership benchmark: check if a slice of IDs forms a quorum of a Membership.
pub fn membership_is_quorum_slice(ids: &[u64], membership: &StoredMembershipOf<UTConfig>) -> bool {
    membership.is_quorum(ids.iter())
}

/// Membership benchmark: check if a BTreeSet of IDs forms a quorum of a Membership.
pub fn membership_is_quorum_btreeset(
    ids: &std::collections::BTreeSet<u64>,
    membership: &StoredMembershipOf<UTConfig>,
) -> bool {
    membership.is_quorum(ids.iter())
}

/// Helper to create a StoredMembership for benchmarks.
pub fn new_membership(node_ids: Vec<std::collections::BTreeSet<u64>>) -> StoredMembershipOf<UTConfig> {
    let m = Membership::<u64, ()>::new_with_defaults(node_ids, None);
    StoredMembershipOf::<UTConfig>::new(None, m)
}
