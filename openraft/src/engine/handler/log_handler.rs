use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::raft_state::LogStateReader;
use crate::LogId;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

/// Handle raft vote related operations
pub(crate) struct LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
}

impl<'x, NID, N> LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Purge log entries upto `upto`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self, upto: LogId<NID>) {
        tracing::info!(upto = display(&upto), "purge_log");
        // TODO: move tests into this mod
        let st = &mut self.state;
        let log_id = Some(&upto);

        if log_id <= st.last_purged_log_id() {
            return;
        }

        st.purge_log(&upto);

        self.output.push_command(Command::PurgeLog { upto });
    }
}

#[cfg(test)]
mod tests {
    mod purge_log_test {

        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::engine::LogIdList;
        use crate::raft_state::LogStateReader;
        use crate::LeaderId;
        use crate::LogId;

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::<u64, ()>::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.state.log_ids = LogIdList::new(vec![log_id(2, 2), log_id(4, 4), log_id(4, 6)]);
            eng.state.next_purge = 3;
            eng
        }

        #[test]
        fn test_purge_log_already_purged() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(1, 1));

            assert_eq!(Some(log_id(2, 2)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(2, 2), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(0, lh.output.commands.len());

            Ok(())
        }

        #[test]
        fn test_purge_log_equal_prev_last_purged() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(2, 2));

            assert_eq!(Some(log_id(2, 2)), lh.state.last_purged_log_id().copied());
            assert_eq!(log_id(2, 2), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(0, lh.output.commands.len());

            Ok(())
        }
        #[test]
        fn test_purge_log_same_leader_as_prev_last_purged() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(2, 3));

            assert_eq!(Some(log_id(2, 3)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(2, 3), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(2, 3) }], lh.output.commands);

            Ok(())
        }

        #[test]
        fn test_purge_log_to_last_key_log() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(4, 4));

            assert_eq!(Some(log_id(4, 4)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(4, 4), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 4) }], lh.output.commands);

            Ok(())
        }

        #[test]
        fn test_purge_log_go_pass_last_key_log() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(4, 5));

            assert_eq!(Some(log_id(4, 5)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(4, 5), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 5) }], lh.output.commands);

            Ok(())
        }

        #[test]
        fn test_purge_log_to_last_log_id() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(4, 6));

            assert_eq!(Some(log_id(4, 6)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(4, 6), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 6)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 6) }], lh.output.commands);

            Ok(())
        }

        #[test]
        fn test_purge_log_go_pass_last_log_id() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(4, 7));

            assert_eq!(Some(log_id(4, 7)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(4, 7), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(4, 7)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 7) }], lh.output.commands);

            Ok(())
        }

        #[test]
        fn test_purge_log_to_higher_leader_lgo() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.purge_log(log_id(5, 7));

            assert_eq!(Some(log_id(5, 7)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(5, 7), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(5, 7)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(5, 7) }], lh.output.commands);

            Ok(())
        }
    }
}
