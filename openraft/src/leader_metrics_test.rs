use crate::leader_metrics::LeaderMetrics;
use crate::leader_metrics::UpdateMatchedLogId;
use crate::testing::DummyConfig;
use crate::versioned::Updatable;
use crate::versioned::Versioned;
use crate::LeaderId;
use crate::LogId;
use crate::MessageSummary;

#[test]
fn test_versioned() -> anyhow::Result<()> {
    let mut a = Versioned::new(LeaderMetrics::<DummyConfig> {
        replication: Default::default(),
    });

    assert_eq!("{ver:0, LeaderMetrics{}}", a.summary());

    // In place update

    a.update(UpdateMatchedLogId {
        target: 1,
        matched: LogId::new(LeaderId::new(1, 2), 3),
    });

    assert_eq!("{ver:1, LeaderMetrics{1:1-2-3}}", a.summary());

    let mut b1 = a.clone();

    // Two instances reference the same data.
    // In place update applies to both instance.

    b1.update(UpdateMatchedLogId {
        target: 1,
        matched: LogId::new(LeaderId::new(1, 2), 5),
    });
    assert_eq!("{ver:1, LeaderMetrics{1:1-2-5}}", a.summary());
    assert_eq!("{ver:2, LeaderMetrics{1:1-2-5}}", b1.summary());

    // In place update is not possible.
    // Fall back to cloned update

    b1.update(UpdateMatchedLogId {
        target: 2,
        matched: LogId::new(LeaderId::new(1, 2), 5),
    });
    assert_eq!("{ver:1, LeaderMetrics{1:1-2-5}}", a.summary());
    assert_eq!("{ver:3, LeaderMetrics{1:1-2-5, 2:1-2-5}}", b1.summary());

    // a and b1 have the same content but not equal, because they reference different data.

    a.update(UpdateMatchedLogId {
        target: 1,
        matched: LogId::new(LeaderId::new(1, 2), 5),
    });
    a.update(UpdateMatchedLogId {
        target: 2,
        matched: LogId::new(LeaderId::new(1, 2), 5),
    });
    assert_eq!("{ver:3, LeaderMetrics{1:1-2-5, 2:1-2-5}}", a.summary());
    assert_eq!("{ver:3, LeaderMetrics{1:1-2-5, 2:1-2-5}}", b1.summary());
    assert_ne!(a, b1);

    // b2 reference the same data as b1.

    let mut b2 = b1.clone();
    b2.update(UpdateMatchedLogId {
        target: 2,
        matched: LogId::new(LeaderId::new(1, 2), 9),
    });
    assert_eq!("{ver:3, LeaderMetrics{1:1-2-5, 2:1-2-9}}", b1.summary());
    assert_eq!("{ver:4, LeaderMetrics{1:1-2-5, 2:1-2-9}}", b2.summary());
    assert_ne!(b1, b2);

    // Two Versioned are equal only when they reference the same data and have the same version.

    b1.update(UpdateMatchedLogId {
        target: 2,
        matched: LogId::new(LeaderId::new(1, 2), 9),
    });
    assert_eq!("{ver:4, LeaderMetrics{1:1-2-5, 2:1-2-9}}", b1.summary());
    assert_eq!("{ver:4, LeaderMetrics{1:1-2-5, 2:1-2-9}}", b2.summary());
    assert_eq!(b1, b2);

    Ok(())
}

#[test]
fn test_versioned_methods() -> anyhow::Result<()> {
    let mut a = Versioned::new(LeaderMetrics::<DummyConfig> {
        replication: Default::default(),
    });

    a.update(UpdateMatchedLogId {
        target: 1,
        matched: LogId::new(LeaderId::new(1, 2), 3),
    });

    assert_eq!("{ver:1, LeaderMetrics{1:1-2-3}}", a.summary());

    assert_eq!(1, a.version());
    assert_eq!("LeaderMetrics{1:1-2-3}", a.data().summary());

    Ok(())
}
