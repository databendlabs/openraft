//! Implement blocking mode write operations for Raft.
//! Blocking mode write API blocks until the write operation is completed,
//! where [`RaftTypeConfig::Responder`] is a [`OneshotResponder`].

use maplit::btreemap;

use crate::core::raft_msg::RaftMsg;
use crate::error::ClientWriteError;
use crate::error::RaftError;
use crate::raft::message::ClientWriteResult;
use crate::raft::responder::OneshotResponder;
use crate::raft::ClientWriteResponse;
use crate::summary::MessageSummary;
use crate::type_config::alias::OneshotReceiverOf;
use crate::AsyncRuntime;
use crate::ChangeMembers;
use crate::Raft;
use crate::RaftTypeConfig;

/// Implement blocking mode write operations those reply on oneshot channel for communication
/// between Raft core and client.
impl<C> Raft<C>
where C: RaftTypeConfig<Responder = OneshotResponder<C>>
{
    /// Propose a cluster configuration change.
    ///
    /// A node in the proposed config has to be a learner, otherwise it fails with LearnerNotFound
    /// error.
    ///
    /// Internally:
    /// - It proposes a **joint** config.
    /// - When the **joint** config is committed, it proposes a uniform config.
    ///
    /// If `retain` is true, then all the members which not exists in the new membership,
    /// will be turned into learners, otherwise will be removed.
    ///
    /// Example of `retain` usage:
    /// If the original membership is {"voter":{1,2,3}, "learners":{}}, and call
    /// `change_membership` with `voters` {3,4,5}, then:
    ///    - If `retain` is `true`, the committed new membership is {"voters":{3,4,5},
    ///      "learners":{1,2}}.
    ///    - Otherwise if `retain` is `false`, then the new membership is {"voters":{3,4,5},
    ///      "learners":{}}, in which the voters not exists in the new membership just be removed
    ///      from the cluster.
    ///
    /// If it loses leadership or crashed before committing the second **uniform** config log, the
    /// cluster is left in the **joint** config.
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership(
        &self,
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C::NodeId, ClientWriteError<C::NodeId, C::Node>>> {
        let changes: ChangeMembers<C::NodeId, C::Node> = members.into();

        tracing::info!(
            changes = debug(&changes),
            retain = display(retain),
            "change_membership: start to commit joint config"
        );

        let (tx, rx) = oneshot_channel::<C>();

        // res is error if membership can not be changed.
        // If no error, it will enter a joint state
        let res = self
            .inner
            .call_core(
                RaftMsg::ChangeMembership {
                    changes: changes.clone(),
                    retain,
                    tx,
                },
                rx,
            )
            .await;

        if let Err(e) = &res {
            tracing::error!("the first step error: {}", e);
        }
        let res = res?;

        tracing::debug!("res of first step: {}", res.summary());

        let (log_id, joint) = (res.log_id.clone(), res.membership.clone().unwrap());

        if joint.get_joint_config().len() == 1 {
            return Ok(res);
        }

        tracing::debug!("committed a joint config: {} {:?}", log_id, joint);
        tracing::debug!("the second step is to change to uniform config: {:?}", changes);

        let (tx, rx) = oneshot_channel::<C>();

        let res = self.inner.call_core(RaftMsg::ChangeMembership { changes, retain, tx }, rx).await;

        if let Err(e) = &res {
            tracing::error!("the second step error: {}", e);
        }
        let res = res?;

        tracing::info!("res of second step of do_change_membership: {}", res.summary());

        Ok(res)
    }

    /// Add a new learner raft node, optionally, blocking until up-to-speed.
    ///
    /// - Add a node as learner into the cluster.
    /// - Setup replication from leader to it.
    ///
    /// If `blocking` is `true`, this function blocks until the leader believes the logs on the new
    /// node is up to date, i.e., ready to join the cluster, as a voter, by calling
    /// `change_membership`.
    ///
    /// If blocking is `false`, this function returns at once as successfully setting up the
    /// replication.
    ///
    /// If the node to add is already a voter or learner, it will still re-add it.
    ///
    /// A `node` is able to store the network address of a node. Thus an application does not
    /// need another store for mapping node-id to ip-addr when implementing the RaftNetwork.
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=display(&id)))]
    pub async fn add_learner(
        &self,
        id: C::NodeId,
        node: C::Node,
        blocking: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C::NodeId, ClientWriteError<C::NodeId, C::Node>>> {
        let (tx, rx) = oneshot_channel::<C>();

        let msg = RaftMsg::ChangeMembership {
            changes: ChangeMembers::AddNodes(btreemap! {id.clone()=>node}),
            retain: true,
            tx,
        };

        let resp = self.inner.call_core(msg, rx).await?;

        if !blocking {
            return Ok(resp);
        }

        if self.inner.id == id {
            return Ok(resp);
        }

        // Otherwise, blocks until the replication to the new learner becomes up to date.

        // The log id of the membership that contains the added learner.
        let membership_log_id = resp.log_id.clone();

        let wait_res = self
            .wait(None)
            .metrics(
                |metrics| match self.check_replication_upto_date(metrics, id.clone(), Some(membership_log_id.clone())) {
                    Ok(_matching) => true,
                    // keep waiting
                    Err(_) => false,
                },
                "wait new learner to become line-rate",
            )
            .await;

        tracing::info!(wait_res = debug(&wait_res), "waiting for replication to new learner");

        Ok(resp)
    }
}

fn oneshot_channel<C>() -> (OneshotResponder<C>, OneshotReceiverOf<C, ClientWriteResult<C>>)
where C: RaftTypeConfig {
    let (tx, rx) = C::AsyncRuntime::oneshot();

    let tx = OneshotResponder::new(tx);

    (tx, rx)
}
