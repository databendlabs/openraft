use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

/// Handle raft vote related operations
pub(crate) struct LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'x mut EngineConfig<NID>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
}

impl<'x, NID, N> LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Purge log entries upto `RaftState.purge_upto()`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self) {
        let st = &mut self.state;
        let purge_upto = st.purge_upto();

        tracing::info!(
            last_purged_log_id = display(st.last_purged_log_id().summary()),
            purge_upto = display(purge_upto.summary()),
            "purge_log"
        );

        if purge_upto <= st.last_purged_log_id() {
            return;
        }

        let upto = *purge_upto.unwrap();

        st.purge_log(&upto);
        self.output.push_command(Command::PurgeLog { upto });
    }

    /// Update the next log id to purge upto, if more logs can be purged, according to configured policy.
    ///
    /// This method is called after building a snapshot, because openraft only purge logs that are already included in
    /// snapshot.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_purge_upto(&mut self) {
        if let Some(purge_upto) = self.calc_purge_upto() {
            debug_assert!(self.state.purge_upto() <= Some(&purge_upto));

            self.state.purge_upto = Some(purge_upto);
        }
    }

    /// Calculate the log id up to which to purge, inclusive.
    ///
    /// Only log included in snapshot will be purged.
    /// It may return None if there is no log to purge.
    ///
    /// `max_keep` specifies the number of applied logs to keep.
    /// `max_keep==0` means every applied log can be purged.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_purge_upto(&self) -> Option<LogId<NID>> {
        let st = &self.state;
        let max_keep = self.config.max_in_snapshot_log_to_keep;
        let batch_size = self.config.purge_batch_size;

        let purge_end = self.state.snapshot_meta.last_log_id.next_index().saturating_sub(max_keep);

        tracing::debug!(
            snapshot_last_log_id = debug(self.state.snapshot_meta.last_log_id),
            max_keep,
            "try purge: (-oo, {})",
            purge_end
        );

        if st.last_purged_log_id().next_index() + batch_size > purge_end {
            tracing::debug!(
                snapshot_last_log_id = debug(self.state.snapshot_meta.last_log_id),
                max_keep,
                last_purged_log_id = display(st.last_purged_log_id().summary()),
                batch_size,
                purge_end,
                "no need to purge",
            );
            return None;
        }

        let log_id = self.state.log_ids.get(purge_end - 1);
        debug_assert!(
            log_id.is_some(),
            "log id not found at {}, engine.state:{:?}",
            purge_end - 1,
            st
        );

        log_id
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
            eng.state.purged_next = 3;
            eng
        }

        #[test]
        fn test_purge_log_already_purged() -> anyhow::Result<()> {
            let mut eng = eng();

            let mut lh = eng.log_handler();
            lh.state.purge_upto = Some(log_id(1, 1));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(2, 2));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(2, 3));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(4, 4));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(4, 5));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(4, 6));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(4, 7));
            lh.purge_log();

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
            lh.state.purge_upto = Some(log_id(5, 7));
            lh.purge_log();

            assert_eq!(Some(log_id(5, 7)), lh.state.last_purged_log_id().copied(),);
            assert_eq!(log_id(5, 7), lh.state.log_ids.key_log_ids()[0],);
            assert_eq!(Some(log_id(5, 7)), lh.state.last_log_id().copied());

            assert_eq!(vec![Command::PurgeLog { upto: log_id(5, 7) }], lh.output.commands);

            Ok(())
        }
    }
    mod calc_purge_upto_test {
        use crate::engine::Engine;
        use crate::engine::LogIdList;
        use crate::LeaderId;
        use crate::LogId;

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 0 },
                index,
            }
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.state.log_ids = LogIdList::new(vec![
                //
                log_id(0, 0),
                log_id(1, 1),
                log_id(3, 3),
                log_id(5, 5),
            ]);
            eng
        }

        #[test]
        fn test_calc_purge_upto() -> anyhow::Result<()> {
            // last_purged_log_id, last_snapshot_log_id, max_keep, want
            // last_applied should not affect the purge
            let cases = vec![
                //
                (None, None, 0, None),
                (None, None, 1, None),
                //
                (None, Some(log_id(1, 1)), 0, Some(log_id(1, 1))),
                (None, Some(log_id(1, 1)), 1, Some(log_id(0, 0))),
                (None, Some(log_id(1, 1)), 2, None),
                //
                (Some(log_id(0, 0)), Some(log_id(1, 1)), 0, Some(log_id(1, 1))),
                (Some(log_id(0, 0)), Some(log_id(1, 1)), 1, None),
                (Some(log_id(0, 0)), Some(log_id(1, 1)), 2, None),
                //
                (None, Some(log_id(3, 4)), 0, Some(log_id(3, 4))),
                (None, Some(log_id(3, 4)), 1, Some(log_id(3, 3))),
                (None, Some(log_id(3, 4)), 2, Some(log_id(1, 2))),
                (None, Some(log_id(3, 4)), 3, Some(log_id(1, 1))),
                (None, Some(log_id(3, 4)), 4, Some(log_id(0, 0))),
                (None, Some(log_id(3, 4)), 5, None),
                //
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 0, Some(log_id(3, 4))),
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 1, Some(log_id(3, 3))),
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 2, None),
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 3, None),
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 4, None),
                (Some(log_id(1, 2)), Some(log_id(3, 4)), 5, None),
            ];

            for (last_purged, snapshot_last_log_id, max_keep, want) in cases {
                let mut eng = eng();
                eng.config.max_in_snapshot_log_to_keep = max_keep;
                eng.config.purge_batch_size = 1;

                if let Some(last_purged) = last_purged {
                    eng.state.log_ids.purge(&last_purged);
                    eng.state.purged_next = last_purged.index + 1;
                }
                eng.state.snapshot_meta.last_log_id = snapshot_last_log_id;
                let got = eng.log_handler().calc_purge_upto();

                assert_eq!(
                    want, got,
                    "case: last_purged: {:?}, snapshot_last_log_id: {:?}, max_keep: {}",
                    last_purged, snapshot_last_log_id, max_keep
                );
            }

            Ok(())
        }
    }
}
