use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use anyhow::Result;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::alias::EntryOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::membership::RaftMembership;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;

use crate::fixtures::TestingVoteExt;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::TestConfig;
use crate::fixtures::custom_membership::cluster_config;

/// Node sets that show which nodes commit an entry and elect a leader under `membership`.
///
/// Node 1 leads first. Its entry `x` waits while node 1 reaches only `partial`, and commits once
/// node 1 reaches `write_quorum`. Then node 1 is cut off, and `candidate`, which applied `x`,
/// campaigns while it reaches only `read_quorum`. A read quorum elects it with advanced leader
/// IDs; with standard leader IDs it needs a vote set that is also a write quorum. Either way, it
/// commits its blank entry only once it reaches `read_and_write_quorum`.
pub struct QuorumScenario<M> {
    pub node_ids: BTreeSet<u64>,
    pub membership: M,
    /// A read quorum that holds node 1 and is no write quorum.
    pub partial: BTreeSet<u64>,
    /// A write quorum that holds node 1.
    pub write_quorum: BTreeSet<u64>,
    /// A node of `write_quorum` other than node 1.
    pub candidate: u64,
    /// A read quorum that holds `candidate` and is no write quorum.
    pub read_quorum: BTreeSet<u64>,
    /// A read quorum and write quorum that holds `candidate`.
    pub read_and_write_quorum: BTreeSet<u64>,
}

impl<M> QuorumScenario<M>
where M: RaftMembership
{
    /// Runs the scenario with type config `C`, whose leader ID type decides which votes elect.
    pub async fn run<C>(&self) -> Result<()>
    where
        C: TestConfig + RaftTypeConfig<D = String, Membership = M>,
        EntryOf<C>: Clone + PartialEq + fmt::Debug,
    {
        let cluster = TestCluster::<C>::start(
            cluster_config(SnapshotPolicy::Never),
            self.node_ids.clone(),
            self.membership.clone(),
        )
        .await?;
        let candidate = self.candidate;
        let isolated = self.cut_outside(1, &BTreeSet::from([1]));

        tracing::info!("--- node 1 wins term 1 and commits its blank entry; every node stores the entry");
        {
            cluster.raft(1).trigger().elect(false).await?;
            cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
            cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
            for node_id in &self.node_ids {
                cluster.raft(*node_id).wait(timeout()).log_index(Some(1), "store the blank entry").await?;
            }
        }

        tracing::info!(
            partial = ?self.partial,
            "--- node 1 reaches only partial, which stores x at index 2 without committing it"
        );
        {
            cluster.cut_links(self.cut_outside(1, &self.partial));
            cluster.raft(1).client_write_ff("x".to_string(), None).await?;
            for node_id in &self.partial {
                cluster.raft(*node_id).wait(timeout()).log_index(Some(2), "store x").await?;
            }
            C::sleep(Duration::from_millis(500)).await;

            let raft = cluster.raft(1);
            let metrics = raft.metrics().borrow_watched().clone();
            assert_eq!(
                Some(log_id::<C>(1, 1, 1)),
                metrics.cluster_committed,
                "partial is no write quorum"
            );
        }

        tracing::info!(
            write_quorum = ?self.write_quorum,
            "--- node 1 reaches write_quorum, which commits and applies x"
        );
        {
            cluster.cut_links(self.cut_outside(1, &self.write_quorum));
            for node_id in &self.write_quorum {
                cluster.raft(*node_id).wait(timeout()).applied_index(Some(2), "apply x").await?;
            }
        }

        tracing::info!("--- cut node 1 off, and wait until its lease runs out");
        {
            cluster.cut_links(isolated.clone());

            // `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
            let leader_lease = Duration::from_millis(cluster.raft(1).config().election_timeout_max);
            C::sleep(leader_lease + Duration::from_millis(200)).await;
        }

        tracing::info!(
            candidate,
            read_quorum = ?self.read_quorum,
            "--- the candidate reaches only read_quorum and campaigns in term 2"
        );
        {
            let mut cut = isolated.clone();
            cut.extend(self.cut_outside(candidate, &self.read_quorum));
            cluster.cut_links(cut);
            cluster.raft(candidate).trigger().elect(false).await?;
        }

        if C::LeaderId::TOTALLY_ORDERED {
            tracing::info!(
                "--- with advanced leader IDs the read quorum elects the candidate, but its blank entry does not commit"
            );
            {
                let raft = cluster.raft(candidate);
                raft.wait(timeout()).state(ServerState::Leader, "the read quorum elects the candidate").await?;
                for node_id in &self.read_quorum {
                    cluster.raft(*node_id).wait(timeout()).log_index(Some(3), "store the blank entry").await?;
                }
                C::sleep(Duration::from_millis(500)).await;

                let metrics = raft.metrics().borrow_watched().clone();
                assert_eq!(
                    Some(log_id::<C>(1, 1, 2)),
                    metrics.last_applied,
                    "read_quorum is no write quorum"
                );
            }
        } else {
            tracing::info!(
                "--- with standard leader IDs the read quorum votes for the candidate, but does not elect it"
            );
            {
                let vote = VoteOf::<C>::from_term_node_id(2, candidate);
                for node_id in &self.read_quorum {
                    cluster.raft(*node_id).wait(timeout()).vote(vote.clone(), "vote for the candidate").await?;
                }
                C::sleep(Duration::from_millis(500)).await;

                let raft = cluster.raft(candidate);
                let metrics = raft.metrics().borrow_watched().clone();
                assert_eq!(ServerState::Candidate, metrics.state, "read_quorum is no write quorum");
            }
        }

        tracing::info!(
            read_and_write_quorum = ?self.read_and_write_quorum,
            "--- the candidate reaches read_and_write_quorum"
        );
        let leader_term = {
            let mut cut = isolated;
            cut.extend(self.cut_outside(candidate, &self.read_and_write_quorum));
            cluster.cut_links(cut);

            if C::LeaderId::TOTALLY_ORDERED {
                2
            } else {
                cluster.raft(candidate).trigger().elect(false).await?;
                let raft = cluster.raft(candidate);
                raft.wait(timeout())
                    .state(ServerState::Leader, "read_and_write_quorum elects the candidate")
                    .await?;
                3
            }
        };

        tracing::info!(
            leader_term,
            "--- read_and_write_quorum applies x and then the blank entry of the candidate"
        );
        {
            let want = vec![
                EntryOf::<C>::new_membership(log_id::<C>(0, 0, 0), self.membership.clone()),
                EntryOf::<C>::new_blank(log_id::<C>(1, 1, 1)),
                EntryOf::<C>::new_normal(log_id::<C>(1, 1, 2), "x".to_string()),
                EntryOf::<C>::new_blank(log_id::<C>(leader_term, candidate, 3)),
            ];
            for node_id in &self.read_and_write_quorum {
                let raft = cluster.raft(*node_id);
                raft.wait(timeout()).applied_index(Some(3), "apply the blank entry of the candidate").await?;
                let applied = cluster.applied(*node_id);
                assert_eq!(want, applied, "node {node_id}");
            }
        }

        Ok(())
    }

    /// Returns the links from `node_id` to every node outside `reachable`.
    fn cut_outside(&self, node_id: u64, reachable: &BTreeSet<u64>) -> BTreeSet<(u64, u64)> {
        let mut links = BTreeSet::new();
        for other in self.node_ids.difference(reachable) {
            links.insert((node_id, *other));
        }
        links
    }
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
