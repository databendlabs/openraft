use crate::testing::DummyConfig as Config;
use crate::vote::leader_id::LeaderId;

#[test]
fn test_leader_id() -> anyhow::Result<()> {
    let l11 = LeaderId::<Config>::new(1, 1);
    let l12 = LeaderId::<Config>::new(1, 2);
    let l21 = LeaderId::<Config>::new(2, 1);

    assert!(l11 < l12);
    assert!(l12 < l21);

    assert_eq!(l12, LeaderId::<Config>::new(1, 2));

    Ok(())
}
