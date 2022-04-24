use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::testing::Config;
use crate::engine::Command;
use crate::engine::Engine;
use crate::entry::EntryRef;
use crate::error::InitializeError;
use crate::error::NotAMembershipEntry;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::EntryPayload;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::Vote;

#[test]
fn test_initialize() -> anyhow::Result<()> {
    let eng = Engine::<u64>::default;

    let log_id0 = LogId {
        leader_id: LeaderId::new(0, 0),
        index: 0,
    };
    let vote0 = Vote::new(0, 0);

    let m12 = || Membership::<u64>::new(vec![btreeset! {1,2}], None);
    let payload = EntryPayload::<Config>::Membership(m12());
    let mut entries = [EntryRef::new(&payload)];

    tracing::info!("--- ok: init empty node 1 with membership(1,2)");
    tracing::info!("--- expect OK result, check output commands and state changes");
    {
        let mut eng = eng();
        eng.id = 1;

        eng.initialize(&mut entries)?;
        eng.update_metrics_flags();

        assert_eq!(Some(log_id0), eng.state.get_log_id(0));
        assert_eq!(None, eng.state.get_log_id(1));
        assert_eq!(Some(log_id0), eng.state.last_log_id);

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: true
            },
            eng.metrics_flags
        );
        assert_eq!(m12(), eng.state.effective_membership.membership);

        assert_eq!(
            vec![
                Command::AppendInputEntries { range: 0..1 },
                Command::UpdateMembership { membership: m12() },
                Command::MoveInputCursorBy { n: 1 },
                Command::UpdateServerState {
                    server_state: ServerState::Candidate
                }
            ],
            eng.commands
        );
    }

    tracing::info!("--- not allowed because of last_log_id");
    {
        let mut eng = eng();
        eng.state.last_log_id = Some(log_id0);

        assert_eq!(
            Err(InitializeError::NotAllowed(NotAllowed {
                last_log_id: Some(log_id0),
                vote: vote0,
            })),
            eng.initialize(&mut entries)
        );
    }

    tracing::info!("--- not allowed because of vote");
    {
        let mut eng = eng();
        eng.state.vote = Vote::new(0, 1);

        assert_eq!(
            Err(InitializeError::NotAllowed(NotAllowed {
                last_log_id: None,
                vote: Vote::new(0, 1),
            })),
            eng.initialize(&mut entries)
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
            eng.initialize(&mut entries)
        );
    }

    tracing::info!("--- log entry is not a membership entry");
    {
        let mut eng = eng();

        let payload = EntryPayload::<Config>::Blank;
        let mut entries = [EntryRef::new(&payload)];

        assert_eq!(
            Err(InitializeError::NotAMembershipEntry(NotAMembershipEntry {})),
            eng.initialize(&mut entries)
        );
    }

    Ok(())
}
