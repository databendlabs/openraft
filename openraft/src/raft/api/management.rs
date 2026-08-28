use std::fmt::Debug;

use display_more::DisplayResultExt;
use maplit::btreemap;
use openraft_macros::since;

use crate::ChangeMembers;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::RaftMetrics;
use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::membership_payloads::MembershipPayloads;
use crate::core::replication_lag;
use crate::entry::RaftPayload;
use crate::errors::ClientWriteError;
use crate::errors::Fatal;
use crate::errors::InitializeError;
use crate::impls::ProgressResponder;
use crate::membership::IntoNodes;
use crate::raft::ChangeMembershipOutcome;
use crate::raft::ChangeMembershipRequest;
use crate::raft::ClientWriteResult;
use crate::raft::Precondition;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::LogIdOf;

/// Provides management APIs for the Raft system.
///
/// This struct contains methods for managing the Raft cluster, including
/// membership changes and node additions.
#[since(version = "0.10.0")]
pub(crate) struct ManagementApi<'a, C>
where C: RaftTypeConfig
{
    inner: &'a RaftInner<C>,
}

impl<'a, C> ManagementApi<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(inner: &'a RaftInner<C>) -> Self {
        Self { inner }
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn initialize<T>(&self, members: T) -> Result<Result<(), InitializeError<C>>, Fatal<C>>
    where T: IntoNodes<C::NodeId, C::Node> + Debug {
        self.inner
            .call_core_oneshot(|tx| RaftMsg::Initialize {
                members: members.into_nodes(),
                tx,
            })
            .await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "info", skip_all)]
    pub(crate) async fn change_membership(
        &self,
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
        preconditions: BatchOf<C, Precondition<C>>,
    ) -> Result<ClientWriteResult<C>, Fatal<C>> {
        let request = ChangeMembershipRequest::new(members, retain).with_preconditions(preconditions);
        let result = self.change_membership_with_payload(request).await?;
        let result = result.map(|outcome| outcome.uniform);
        Ok(result)
    }

    pub(crate) async fn change_membership_with_payload(
        &self,
        request: ChangeMembershipRequest<C>,
    ) -> Result<Result<ChangeMembershipOutcome<C>, ClientWriteError<C>>, Fatal<C>> {
        let (changes, retain, preconditions, payload) = request.into_parts();
        let (joint_payload, uniform_payload) = match payload {
            Some((joint, uniform)) => (joint, uniform),
            None => (C::Payload::blank(), C::Payload::blank()),
        };

        tracing::info!(
            "change_membership: start to commit joint config: changes: {:?}, retain: {}",
            changes,
            retain
        );

        let (tx, rx) = ProgressResponder::<C, _>::complete_only();

        tracing::debug!("change_membership: start",);

        // `RaftCore` computes the membership, so only `RaftCore` can tell whether this change
        // needs a joint membership. It spends `joint_payload` on a joint membership and
        // `uniform_payload` on a uniform one, and returns the payload it did not spend.
        let (unused_tx, unused_rx) = C::oneshot();
        let payloads = MembershipPayloads::JointOrUniform {
            joint: joint_payload,
            uniform: uniform_payload,
            unused_tx,
        };

        // res is error if membership cannot be changed.
        // If no error, it will enter a joint state
        let client_write_result = self
            .inner
            .call_core(
                RaftMsg::ChangeMembership {
                    changes: changes.clone(),
                    payloads,
                    retain,
                    preconditions: Batch::of(preconditions.as_ref().iter().cloned()),
                    tx,
                },
                rx,
            )
            .await?;

        tracing::debug!(
            "change_membership: client_write_result: {}",
            client_write_result.display()
        );

        let resp = match client_write_result {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("the first step error: {}", e);
                return Ok(Err(e));
            }
        };

        tracing::debug!("res of first step: {}", resp);

        let (log_id, membership) = (&resp.log_id, resp.membership.clone().unwrap());

        if !membership.is_joint() {
            let outcome = ChangeMembershipOutcome {
                joint: None,
                uniform: resp,
            };
            return Ok(Ok(outcome));
        }

        // The first step spent `joint_payload` on the joint entry, so the payload it returned is
        // `uniform_payload`, for the entry this second step writes.
        let uniform_payload = self.inner.recv_msg(unused_rx).await?;

        tracing::debug!("committed a joint config: {} {:?}", log_id, membership);
        tracing::debug!("the second step is to change to uniform config: {:?}", changes);

        // The last membership config log ID is changed because we have proposed a new membership config
        // log. Therefore, we need to remove the previous membership log ID precondition and add a new
        // precondition.
        // For the same reason, LastLogId is changed and its precondition must be removed.
        let preconditions = preconditions
            .into_iter()
            .filter_map(|x| match x {
                Precondition::LastMembershipLogId { .. } => None,
                Precondition::LastLogId { .. } => None,
                // A leader change between the two steps must still abort the second step.
                Precondition::CommittedLeaderId { .. } => Some(x),
            })
            .chain([Precondition::LastMembershipLogId {
                last_membership_log_id: Some(log_id.clone()),
            }]);
        let preconditions = Batch::of(preconditions);

        let (tx, rx) = ProgressResponder::<C, _>::complete_only();

        // The second step, send a NOOP change to flatten the joint config.
        let changes = ChangeMembers::AddVoterIds(Default::default());
        let client_write_result = self
            .inner
            .call_core(
                RaftMsg::ChangeMembership {
                    changes,
                    payloads: MembershipPayloads::Uniform(uniform_payload),
                    retain,
                    preconditions,
                    tx,
                },
                rx,
            )
            .await?;

        tracing::info!(
            "result of second step of change_membership: {}",
            client_write_result.display()
        );

        if let Err(e) = &client_write_result {
            tracing::error!("the second step error: {}", e);
        }

        let outcome = client_write_result.map(|uniform| ChangeMembershipOutcome {
            joint: Some(resp),
            uniform,
        });
        Ok(outcome)
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "info", skip_all)]
    pub(crate) async fn append_membership(
        &self,
        membership: Membership<C::NodeId, C::Node>,
        payload: C::Payload,
        preconditions: BatchOf<C, Precondition<C>>,
    ) -> Result<ClientWriteResult<C>, Fatal<C>> {
        tracing::info!("append_membership: proposed membership: {}", membership);

        // Bind the membership into the payload here, so the core has exactly one value to
        // validate and to write: the payload it receives.
        let payload = payload.with_membership(membership);

        let (tx, rx) = ProgressResponder::<C, _>::complete_only();

        let msg = RaftMsg::AppendMembership {
            payload,
            preconditions,
            tx,
        };

        self.inner.call_core(msg, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=display(&id)))]
    pub(crate) async fn add_learner(
        &self,
        id: C::NodeId,
        node: C::Node,
        blocking: bool,
    ) -> Result<ClientWriteResult<C>, Fatal<C>> {
        let (tx, rx) = ProgressResponder::<C, _>::complete_only();

        let msg = RaftMsg::ChangeMembership {
            changes: ChangeMembers::AddNodes(btreemap! {id.clone()=>node}),
            payloads: MembershipPayloads::Uniform(C::Payload::blank()),
            retain: true,
            preconditions: Batch::of([]),
            tx,
        };

        let client_write_result = self.inner.call_core(msg, rx).await?;

        let resp = match client_write_result {
            Ok(x) => x,
            Err(e) => return Ok(Err(e)),
        };

        if !blocking {
            return Ok(Ok(resp));
        }

        if self.inner.id == id {
            return Ok(Ok(resp));
        }

        // Otherwise, blocks until the replication to the new learner becomes up to date.

        // The log id of the membership that contains the added learner.
        let membership_log_id = &resp.log_id;

        let wait_res = self
            .inner
            .wait(None)
            .metrics(
                |metrics| match self.check_replication_upto_date(metrics, &id, Some(membership_log_id)) {
                    Ok(_matching) => true,
                    // keep waiting
                    Err(_) => false,
                },
                "wait new learner to become line-rate",
            )
            .await;

        tracing::info!(
            "waiting for replication to new learner: wait_res: {}",
            wait_res.display()
        );

        Ok(Ok(resp))
    }

    #[since(version = "0.10.0")]
    fn check_replication_upto_date(
        &self,
        metrics: &RaftMetrics<C>,
        node_id: &C::NodeId,
        membership_log_id: Option<&LogIdOf<C>>,
    ) -> Result<Option<LogIdOf<C>>, ()> {
        if metrics.membership_config.log_id().as_ref() < membership_log_id {
            // Waiting for the latest metrics to report.
            return Err(());
        }

        if metrics.membership_config.membership().get_node(node_id).is_none() {
            // This learner has been removed.
            return Ok(None);
        }

        let repl = match &metrics.replication {
            None => {
                // This node is no longer a leader.
                return Ok(None);
            }
            Some(x) => x,
        };

        let replication_metrics = repl;
        let target_metrics = match replication_metrics.get(node_id) {
            None => {
                // Maybe replication is not reported yet. Keep waiting.
                return Err(());
            }
            Some(x) => x,
        };

        let matched = target_metrics.clone();

        let distance = replication_lag(&matched.index(), &metrics.last_log_index);

        if distance <= self.inner.config.replication_lag_threshold {
            // replication became up to date.
            return Ok(matched);
        }

        // Not up to date, keep waiting.
        Err(())
    }
}
