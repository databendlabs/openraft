use crate::raft_state::accepted::Accepted;
use crate::CommittedLeaderId;
use crate::LeaderId;
use crate::LogId;

#[test]
fn test_accepted() -> anyhow::Result<()> {
    let a = Accepted::new(LeaderId::new(5, 10), Some(LogId::new(CommittedLeaderId::new(6, 2), 3)));
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(6, 2), 3)),
        a.last_accepted_log_id(&LeaderId::new(5, 10)),
    );
    assert_eq!(None, a.last_accepted_log_id(&LeaderId::new(6, 10)));
    assert_eq!(None, a.last_accepted_log_id(&LeaderId::new(4, 10)));

    Ok(())
}
