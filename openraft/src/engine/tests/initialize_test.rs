use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::entry::RaftEntry;
use crate::error::InitializeError;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::raft::VoteRequest;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::vote::CommittedLeaderId;
use crate::Entry;
use crate::LogId;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

#[test]
fn test_initialize_single_node() -> anyhow::Result<()> {
    let eng = || {
        let mut eng = Engine::<UTConfig>::default();
        eng.state.enable_validation(false); // Disable validation for incomplete state

        eng.state.server_state = eng.calc_server_state();
        eng
    };

    let log_id0 = LogId {
        leader_id: CommittedLeaderId::new(0, 0),
        index: 0,
    };

    let m1 = || Membership::<u64, ()>::new(vec![btreeset! {1}], None);
    let entry = Entry::<UTConfig>::new_membership(LogId::default(), m1());

    tracing::info!("--- ok: init empty node 1 with membership(1,2)");
    tracing::info!("--- expect OK result, check output commands and state changes");
    {
        let mut eng = eng();
        eng.config.id = 1;

        eng.initialize(entry)?;

        assert_eq!(Some(log_id0), eng.state.get_log_id(0));
        assert_eq!(Some(log_id(1, 1, 1)), eng.state.get_log_id(1));
        assert_eq!(Some(&log_id(1, 1, 1)), eng.state.last_log_id());

        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert_eq!(&m1(), eng.state.membership_state.effective().membership());

        assert_eq!(
            vec![
                Command::AppendEntry {
                    entry: Entry::<UTConfig>::new_membership(LogId::default(), m1())
                },
                // When update the effective membership, the engine set it to Follower.
                // But when initializing, it will switch to Candidate at once, in the last output
                // command.
                Command::SaveVote { vote: Vote::new(1, 1) },
                // TODO: duplicated SaveVote: one is emitted by elect(), the second is emitted when
                // the node becomes       leader.
                Command::SaveVote {
                    vote: Vote::new_committed(1, 1),
                },
                Command::BecomeLeader,
                Command::RebuildReplicationStreams { targets: vec![] },
                Command::AppendEntry {
                    entry: Entry::<UTConfig>::new_blank(log_id(1, 1, 1))
                },
                Command::ReplicateCommitted {
                    committed: Some(LogId {
                        leader_id: CommittedLeaderId::new(1, 1),
                        index: 1,
                    },),
                },
                Command::Commit {
                    seq: 1,
                    already_committed: None,
                    upto: LogId {
                        leader_id: CommittedLeaderId::new(1, 1),
                        index: 1,
                    },
                },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}

#[test]
fn test_initialize() -> anyhow::Result<()> {
    let eng = || {
        let mut eng = Engine::<UTConfig>::default();
        eng.state.enable_validation(false); // Disable validation for incomplete state

        eng.state.server_state = eng.calc_server_state();
        eng
    };

    let log_id0 = LogId {
        leader_id: CommittedLeaderId::new(0, 0),
        index: 0,
    };

    let m12 = || Membership::<u64, ()>::new(vec![btreeset! {1,2}], None);
    let entry = || Entry::<UTConfig>::new_membership(LogId::default(), m12());

    tracing::info!("--- ok: init empty node 1 with membership(1,2)");
    tracing::info!("--- expect OK result, check output commands and state changes");
    {
        let mut eng = eng();
        eng.config.id = 1;

        eng.initialize(entry())?;

        assert_eq!(Some(log_id0), eng.state.get_log_id(0));
        assert_eq!(None, eng.state.get_log_id(1));
        assert_eq!(Some(&log_id0), eng.state.last_log_id());

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(&m12(), eng.state.membership_state.effective().membership());

        assert_eq!(
            vec![
                Command::AppendEntry {
                    entry: Entry::new_membership(LogId::default(), m12())
                },
                // When update the effective membership, the engine set it to Follower.
                // But when initializing, it will switch to Candidate at once, in the last output
                // command.
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest {
                        vote: Vote::new(1, 1),
                        last_log_id: Some(LogId {
                            leader_id: CommittedLeaderId::new(0, 0),
                            index: 0,
                        },),
                    },
                },
            ],
            eng.output.take_commands()
        );
    }

    tracing::info!("--- not allowed because of last_log_id");
    {
        let mut eng = eng();
        eng.state.log_ids = LogIdList::new(vec![log_id0]);

        assert_eq!(
            Err(InitializeError::NotAllowed(NotAllowed {
                last_log_id: Some(log_id0),
                vote: Vote::default(),
            })),
            eng.initialize(entry())
        );
    }

    tracing::info!("--- not allowed because of vote");
    {
        let mut eng = eng();
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(0, 1));

        assert_eq!(
            Err(InitializeError::NotAllowed(NotAllowed {
                last_log_id: None,
                vote: Vote::new(0, 1),
            })),
            eng.initialize(entry())
        );
    }

    tracing::info!("--- node id 0 is not in membership");
    {
        let mut eng = eng();

        assert_eq!(
            Err(InitializeError::NotInMembers(NotInMembers {
                node_id: 0,
                membership: m12()
            })),
            eng.initialize(entry())
        );
    }

    Ok(())
}
