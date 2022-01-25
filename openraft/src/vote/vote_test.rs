use crate::Vote;

#[test]
fn test_voting_state() -> anyhow::Result<()> {
    let v1u0 = Vote::new_uncommitted(1, None);
    let v1u1 = Vote::new_uncommitted(1, Some(1));
    let v1u2 = Vote::new_uncommitted(1, Some(2));
    let v2u1 = Vote::new_uncommitted(2, Some(1));

    let v1c1 = Vote::new_committed(1, 1);
    let v1c2 = Vote::new_committed(1, 2);
    let v2c1 = Vote::new_committed(2, 1);

    let yes = true;
    let n__ = false;

    assert_eq!(yes, v1u0 <= v1u0);
    assert_eq!(yes, v1u0 <= v1u1);
    assert_eq!(yes, v1u0 <= v1u2);
    assert_eq!(yes, v1u0 <= v2u1);
    assert_eq!(yes, v1u0 <= v1c1);
    assert_eq!(yes, v1u0 <= v1c2);
    assert_eq!(yes, v1u0 <= v2c1);

    assert_eq!(n__, v1u1 <= v1u0);
    assert_eq!(yes, v1u1 <= v1u1);
    assert_eq!(n__, v1u1 <= v1u2);
    assert_eq!(yes, v1u1 <= v2u1);
    assert_eq!(yes, v1u1 <= v1c1);
    assert_eq!(yes, v1u1 <= v1c2);
    assert_eq!(yes, v1u1 <= v2c1);

    assert_eq!(n__, v1u2 <= v1u0);
    assert_eq!(n__, v1u2 <= v1u1);
    assert_eq!(yes, v1u2 <= v1u2);
    assert_eq!(yes, v1u2 <= v2u1);
    assert_eq!(yes, v1u2 <= v1c1);
    assert_eq!(yes, v1u2 <= v1c2);
    assert_eq!(yes, v1u2 <= v2c1);

    assert_eq!(n__, v2u1 <= v1u0);
    assert_eq!(n__, v2u1 <= v1u1);
    assert_eq!(n__, v2u1 <= v1u2);
    assert_eq!(yes, v2u1 <= v2u1);
    assert_eq!(n__, v2u1 <= v1c1);
    assert_eq!(n__, v2u1 <= v1c2);
    assert_eq!(yes, v2u1 <= v2c1);

    assert_eq!(n__, v1c1 <= v1u0);
    assert_eq!(n__, v1c1 <= v1u1);
    assert_eq!(n__, v1c1 <= v1u2);
    assert_eq!(yes, v1c1 <= v2u1);
    assert_eq!(yes, v1c1 <= v1c1);
    // assert_eq!(n, v1c1 <= v1c2); // panic
    assert_eq!(yes, v1c1 <= v2c1);

    assert_eq!(n__, v1c2 <= v1u0);
    assert_eq!(n__, v1c2 <= v1u1);
    assert_eq!(n__, v1c2 <= v1u2);
    assert_eq!(yes, v1c2 <= v2u1);
    // assert_eq!(n, v1c2 <= v1c1); // panic
    assert_eq!(yes, v1c2 <= v1c2);
    assert_eq!(yes, v1c2 <= v2c1);

    assert_eq!(n__, v2c1 <= v1u0);
    assert_eq!(n__, v2c1 <= v1u1);
    assert_eq!(n__, v2c1 <= v1u2);
    assert_eq!(n__, v2c1 <= v2u1);
    assert_eq!(n__, v2c1 <= v1c1);
    assert_eq!(n__, v2c1 <= v1c2);
    assert_eq!(yes, v2c1 <= v2c1);
    Ok(())
}
