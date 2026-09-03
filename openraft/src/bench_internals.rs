//! Crate internals exposed to the criterion benchmarks in `openraft/benches/`.
//!
//! Benchmarks are compiled as a separate crate and cannot reach crate-private items. This module
//! holds only what cannot be spelled from outside the crate; how a benchmark drives these types,
//! and what it measures, belongs to `openraft/benches/`.
//!
//! Gated behind the `bench` feature; not part of the public API.

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Engine;
pub use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
pub use crate::quorum::QuorumSet;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::PayloadOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

/// An `Engine` that is an established leader, for benchmarking `leader_append_entries()`.
pub struct BenchEngine(Engine<UTConfig>);

impl BenchEngine {
    /// Build a leader with committed membership `{0, 1}` and effective membership `{2, 3}`.
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

        Self(eng)
    }

    /// Append `payloads` as the leader.
    #[inline]
    pub fn append<I>(&mut self, payloads: I)
    where I: IntoIterator<Item = PayloadOf<UTConfig>> + AsRef<[PayloadOf<UTConfig>]> {
        self.0.try_leader_handler().unwrap().leader_append_entries(payloads);
    }

    /// Drop the commands [`Self::append()`] has queued, which would otherwise grow without bound.
    #[inline]
    pub fn clear_commands(&mut self) {
        self.0.output.clear_commands();
    }
}
