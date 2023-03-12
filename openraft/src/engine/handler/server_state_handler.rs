use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::ServerState;

/// Handle raft server-state related operations
pub(crate) struct ServerStateHandler<'st, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'st EngineConfig<NID>,
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) output: &'st mut EngineOutput<NID, N>,
}

impl<'st, NID, N> ServerStateHandler<'st, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Re-calculate the server-state, if it changed, update the `server_state` field and dispatch
    /// commands to inform a runtime.
    pub(crate) fn update_server_state_if_changed(&mut self) {
        let server_state = self.state.calc_server_state(&self.config.id);

        tracing::debug!(
            id = display(self.config.id),
            prev_server_state = debug(self.state.server_state),
            server_state = debug(server_state),
            "update_server_state_if_changed"
        );

        if self.state.server_state == server_state {
            return;
        }

        let was_leader = self.state.server_state == ServerState::Leader;
        let is_leader = server_state == ServerState::Leader;

        if !was_leader && is_leader {
            self.output.push_command(Command::BecomeLeader);
        } else if was_leader && !is_leader {
            self.output.push_command(Command::QuitLeader);
        } else {
            // nothing to do
        }

        self.state.server_state = server_state;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maplit::btreeset;
    use pretty_assertions::assert_eq;
    use tokio::time::Instant;

    use crate::engine::Command;
    use crate::engine::Engine;
    use crate::testing::log_id;
    use crate::utime::UTime;
    use crate::EffectiveMembership;
    use crate::Membership;
    use crate::MembershipState;
    use crate::ServerState;
    use crate::Vote;

    fn m01() -> Membership<u64, ()> {
        Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
    }

    fn m123() -> Membership<u64, ()> {
        Membership::<u64, ()>::new(vec![btreeset! {1,2,3}], None)
    }

    fn eng() -> Engine<u64, ()> {
        let mut eng = Engine::default();
        eng.state.enable_validate = false; // Disable validation for incomplete state

        eng.config.id = 2;
        eng.state.vote = UTime::new(Instant::now(), Vote::new_committed(2, 2));
        eng.state.membership_state = MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m123())),
        );
        eng.state.server_state = eng.state.calc_server_state(&eng.config.id);

        eng
    }
    #[test]
    fn test_update_server_state_if_changed() -> anyhow::Result<()> {
        //
        let mut eng = eng();
        let mut ssh = eng.server_state_handler();

        // Leader become follower
        {
            assert_eq!(ServerState::Leader, ssh.state.server_state);

            ssh.output.commands = vec![];
            ssh.state.vote = UTime::new(Instant::now(), Vote::new(2, 100));
            ssh.update_server_state_if_changed();

            assert_eq!(ServerState::Follower, ssh.state.server_state);
            assert_eq!(
                vec![
                    //
                    Command::QuitLeader,
                ],
                ssh.output.commands
            );
        }

        // TODO(3): add more test,
        //          after migrating to the no-step-down leader:
        //          A leader keeps working after it is removed from the voters.
        Ok(())
    }
}
