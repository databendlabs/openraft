use maplit::btreeset;

use crate::quorum::coherent::Coherent;
use crate::quorum::coherent::FindCoherent;

#[test]
fn test_is_coherent() -> anyhow::Result<()> {
    let s123 = || btreeset! {1,2,3};
    let s345 = || btreeset! {3,4,5};
    let s789 = || btreeset! {7,8,9};

    let j123 = vec![s123()];
    let j345 = vec![s345()];
    let j123_345 = vec![s123(), s345()];
    let j345_789 = vec![s345(), s789()];

    // Two joint configs are coherent iff they share at least one config.
    assert!(j123.is_coherent_with(&j123));
    assert!(!j123.is_coherent_with(&j345));
    assert!(j123.is_coherent_with(&j123_345));
    assert!(!j123.is_coherent_with(&j345_789));

    assert!(!j345.is_coherent_with(&j123));
    assert!(j345.is_coherent_with(&j345));
    assert!(j345.is_coherent_with(&j123_345));
    assert!(j345.is_coherent_with(&j345_789));

    assert!(j123_345.is_coherent_with(&j123));
    assert!(j123_345.is_coherent_with(&j345));
    assert!(j123_345.is_coherent_with(&j123_345));
    assert!(j123_345.is_coherent_with(&j345_789));

    assert!(!j345_789.is_coherent_with(&j123));
    assert!(j345_789.is_coherent_with(&j345));
    assert!(j345_789.is_coherent_with(&j123_345));
    assert!(j345_789.is_coherent_with(&j345_789));

    Ok(())
}

#[test]
fn test_find_coherent() -> anyhow::Result<()> {
    let s1 = || btreeset! {1,2,3};
    let s2 = || btreeset! {3,4,5};
    let s3 = || btreeset! {7,8,9};

    let j1 = vec![s1()];
    let j2 = vec![s2()];
    let j12 = vec![s1(), s2()];
    let j23 = vec![s2(), s3()];

    assert_eq!(j1, j1.find_coherent(s1()));
    assert_eq!(j12, j1.find_coherent(s2()));
    assert_eq!(j1, j12.find_coherent(s1()));
    assert_eq!(j2, j12.find_coherent(s2()));
    assert_eq!(j23, j12.find_coherent(s3()));

    Ok(())
}
