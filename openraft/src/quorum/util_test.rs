use crate::quorum::util::majority_of;

#[test]
fn test_majority_of() -> anyhow::Result<()> {
    assert_eq!(1, majority_of(0));
    assert_eq!(1, majority_of(1));
    assert_eq!(2, majority_of(2));
    assert_eq!(2, majority_of(3));
    assert_eq!(3, majority_of(4));
    assert_eq!(3, majority_of(5));
    assert_eq!(4, majority_of(6));
    assert_eq!(4, majority_of(7));

    Ok(())
}
