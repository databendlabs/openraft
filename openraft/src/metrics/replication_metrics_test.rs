use crate::metrics::ReplicationMetrics;
use crate::metrics::UpdateMatchedLogId;
use crate::versioned::Updatable;
use crate::versioned::Versioned;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::MessageSummary;

#[test]
fn test_versioned() -> anyhow::Result<()> {
    #[allow(clippy::redundant_closure)]
    let clid = |term, node_id| CommittedLeaderId::new(term, node_id);

    let mut a = Versioned::new(ReplicationMetrics::<u64> {
        replication: Default::default(),
    });

    assert_eq!("{ver:0, LeaderMetrics{}}", a.summary());

    // In place update

    a.update(UpdateMatchedLogId {
        target: 1,
        matching: LogId::new(clid(1, 2), 3),
    });

    assert_eq!(format!("{{ver:1, LeaderMetrics{{1:{}-3}}}}", clid(1, 2)), a.summary());

    let mut b1 = a.clone();

    // Two instances reference the same data.
    // In place update applies to both instance.

    b1.update(UpdateMatchedLogId {
        target: 1,
        matching: LogId::new(clid(1, 2), 5),
    });

    assert_eq!(format!("{{ver:1, LeaderMetrics{{1:{}-5}}}}", clid(1, 2)), a.summary());
    assert_eq!(format!("{{ver:2, LeaderMetrics{{1:{}-5}}}}", clid(1, 2)), b1.summary());

    // In place update is not possible.
    // Fall back to cloned update

    b1.update(UpdateMatchedLogId {
        target: 2,
        matching: LogId::new(clid(1, 2), 5),
    });
    assert_eq!(format!("{{ver:1, LeaderMetrics{{1:{}-5}}}}", clid(1, 2)), a.summary());
    assert_eq!(
        format!("{{ver:3, LeaderMetrics{{1:{}-5, 2:{}-5}}}}", clid(1, 2), clid(1, 2)),
        b1.summary()
    );

    // a and b1 have the same content but not equal, because they reference different data.

    a.update(UpdateMatchedLogId {
        target: 1,
        matching: LogId::new(clid(1, 2), 5),
    });
    a.update(UpdateMatchedLogId {
        target: 2,
        matching: LogId::new(clid(1, 2), 5),
    });
    assert_eq!(
        format!("{{ver:3, LeaderMetrics{{1:{}-5, 2:{}-5}}}}", clid(1, 2), clid(1, 2)),
        a.summary()
    );
    assert_eq!(
        format!("{{ver:3, LeaderMetrics{{1:{}-5, 2:{}-5}}}}", clid(1, 2), clid(1, 2)),
        b1.summary()
    );
    assert_ne!(a, b1);

    // b2 reference the same data as b1.

    let mut b2 = b1.clone();
    b2.update(UpdateMatchedLogId {
        target: 2,
        matching: LogId::new(clid(1, 2), 9),
    });
    assert_eq!(
        format!("{{ver:3, LeaderMetrics{{1:{}-5, 2:{}-9}}}}", clid(1, 2), clid(1, 2)),
        b1.summary()
    );
    assert_eq!(
        format!("{{ver:4, LeaderMetrics{{1:{}-5, 2:{}-9}}}}", clid(1, 2), clid(1, 2)),
        b2.summary()
    );
    assert_ne!(b1, b2);

    // Two Versioned are equal only when they reference the same data and have the same version.

    b1.update(UpdateMatchedLogId {
        target: 2,
        matching: LogId::new(clid(1, 2), 9),
    });
    assert_eq!(
        format!("{{ver:4, LeaderMetrics{{1:{}-5, 2:{}-9}}}}", clid(1, 2), clid(1, 2)),
        b1.summary()
    );
    assert_eq!(
        format!("{{ver:4, LeaderMetrics{{1:{}-5, 2:{}-9}}}}", clid(1, 2), clid(1, 2)),
        b2.summary()
    );
    assert_eq!(b1, b2);

    Ok(())
}

#[test]
fn test_versioned_methods() -> anyhow::Result<()> {
    #[allow(clippy::redundant_closure)]
    let clid = |term, node_id| CommittedLeaderId::new(term, node_id);

    let mut a = Versioned::new(ReplicationMetrics::<u64> {
        replication: Default::default(),
    });

    a.update(UpdateMatchedLogId {
        target: 1,
        matching: LogId::new(CommittedLeaderId::new(1, 2), 3),
    });

    assert_eq!(format!("{{ver:1, LeaderMetrics{{1:{}-3}}}}", clid(1, 2)), a.summary());

    assert_eq!(1, a.version());
    assert_eq!(format!("LeaderMetrics{{1:{}-3}}", clid(1, 2)), a.data().summary());

    Ok(())
}
