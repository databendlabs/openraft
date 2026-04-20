//! Raft safety-property checker for turmoil-based simulation.
//!
//! # Authoritative references
//!
//! - `Paper §X` — Ongaro & Ousterhout, *In Search of an Understandable Consensus Algorithm*, USENIX
//!   ATC '14 (Figure 3.2 lists Raft's five core safety properties).
//! - `Dissertation §X` — Ongaro, *Consensus: Bridging Theory and Practice*, Stanford 2014.
//! - `Vanlightly` — Jack Vanlightly's model-checking TLA+ spec at [raft-tlaplus], which states the
//!   invariants as explicit TLA+ formulas.
//!
//! [raft-tlaplus]: https://github.com/Vanlightly/raft-tlaplus/blob/main/specifications/standard-raft/Raft.tla
//!
//! | Property                | Paper §     | TLA+ invariant                   | Module                   |
//! |-------------------------|-------------|----------------------------------|--------------------------|
//! | Election Safety         | §3.6.3      | (implicit)                       | [`election_safety`]      |
//! | Log Matching            | §3.5        | `NoLogDivergence`                | [`log_matching`]         |
//! | Leader Completeness     | §3.6.3      | `LeaderHasAllAckedValues`        | [`leader_completeness`]  |
//! | State Machine Safety    | §3.6.3      | (implicit)                       | [`state_machine_safety`] |
//! | Committed On Quorum     | —           | `CommittedEntriesReachMajority`  | [`committed_on_quorum`]  |
//! | Committed Immutable     | derived     | (history-based)                  | [`committed_immutable`]  |
//! | State Ordering          | —           | (implementation sanity)          | [`state_ordering`]       |
//! | Monotonic Term          | §3.3        | `MonotonicTerm`                  | [`monotonic`]            |
//! | Monotonic CommitIndex   | §3.4        | `MonotonicCommitIndex`           | [`monotonic`]            |
//! | Monotonic AppliedIndex  | derived     | (follows `applied ≤ committed`)  | [`monotonic`]            |
//! | Monotonic Vote          | §3.3        | `MonotonicVote`                  | [`monotonic`]            |
//!
//! Leader Append-Only (Paper §3.6.3) is not checked directly: its safety
//! content for committed entries is already covered by Log Matching +
//! Committed Immutable + Leader Completeness.
//!
//! # Leader ID modes
//!
//! Openraft supports two leader-id schemes:
//!
//! - `leader_id_std`: at most one leader per term (textbook Raft). `CommittedLeaderId` is just
//!   `term`.
//! - `leader_id_adv` (openraft default, used by this crate): `LeaderId = (term, node_id)` is
//!   totally ordered, so multiple leaders may co-exist in the same term as long as their `node_id`s
//!   differ. `CommittedLeaderId` is also `(term, node_id)`.
//!
//! All identity comparisons in this module use `CommittedLeaderId` — never
//! just `term` — so the same check is correct under both modes: advanced
//! mode's "different node_ids in the same term" case simply is not flagged.

pub mod committed_immutable;
pub mod committed_on_quorum;
pub mod election_safety;
pub mod leader_completeness;
pub mod log_matching;
pub mod monotonic;
pub mod state_machine_safety;
pub mod state_ordering;
pub mod violation;

#[cfg(test)]
mod tests;

pub use violation::InvariantViolation;

use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// The committed leader id type for this test config.
///
/// - In `leader_id_std` mode it is just `term`.
/// - In `leader_id_adv` mode (openraft default, what this crate uses) it is `(term, node_id)`.
///
/// Using `CLeaderId` as identity — instead of raw `term` — is what lets the
/// same check run correctly in both modes: advanced mode legitimately allows
/// different `node_id`s to lead in the same `term`, so equality must include
/// `node_id`.
pub type CLeaderId = openraft::type_config::alias::CommittedLeaderIdOf<crate::typ::TypeConfig>;

/// Result of one invariant check pass.
#[derive(Debug, Default)]
pub struct InvariantCheckResult {
    pub violations: Vec<InvariantViolation>,
}

/// Stateful invariant checker.
///
/// Retains per-tick history so temporal invariants (Committed Immutable and
/// the Monotonic family) can be checked. A single instance is meant to be
/// driven across every tick of a simulation.
#[derive(Default)]
pub struct InvariantChecker {
    witnesses: committed_immutable::Witness,
    monotonic: monotonic::MonotonicHistory,
}

impl InvariantChecker {
    /// Check every invariant against `snapshots`, updating temporal state.
    pub fn check(&mut self, snapshots: &[(NodeId, FullNodeSnapshot)]) -> InvariantCheckResult {
        let mut violations = Vec::new();

        // --- Single-tick per-node checks ---
        for (id, s) in snapshots {
            state_ordering::check(*id, s, &mut violations);
        }

        // --- Single-tick cross-node checks ---
        election_safety::check(snapshots, &mut violations);
        leader_completeness::check(snapshots, &mut violations);
        committed_on_quorum::check(snapshots, &mut violations);

        // --- Pairwise cross-node checks ---
        for i in 0..snapshots.len() {
            for j in (i + 1)..snapshots.len() {
                log_matching::check(&snapshots[i], &snapshots[j], &mut violations);
                state_machine_safety::check(&snapshots[i], &snapshots[j], &mut violations);
            }
        }

        // --- Cross-tick checks (must update internal state) ---
        self.witnesses.check_and_record(snapshots, &mut violations);
        self.monotonic.check_and_record(snapshots, &mut violations);

        InvariantCheckResult { violations }
    }
}
