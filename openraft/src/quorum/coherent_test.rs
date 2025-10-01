use maplit::btreeset;

use crate::quorum::Joint;
use crate::quorum::coherent::Coherent;
use crate::quorum::coherent::FindCoherent;
use crate::quorum::joint::AsJoint;

#[test]
fn test_is_coherent_vec() -> anyhow::Result<()> {
    let s123 = || btreeset! {1,2,3};
    let s345 = || btreeset! {3,4,5};
    let s789 = || btreeset! {7,8,9};

    let j123 = Joint::from(vec![s123()]);
    let j345 = Joint::from(vec![s345()]);
    let j123_345 = Joint::from(vec![s123(), s345()]);
    let j345_789 = Joint::from(vec![s345(), s789()]);

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
fn test_is_coherent_slice() -> anyhow::Result<()> {
    let s123 = || btreeset! {1,2,3};
    let s345 = || btreeset! {3,4,5};
    let s789 = || btreeset! {7,8,9};

    let v123 = vec![s123()];
    let v345 = vec![s345()];
    let v123_345 = vec![s123(), s345()];
    let v345_789 = vec![s345(), s789()];

    let j123 = v123.as_joint();
    let j345 = v345.as_joint();
    let j123_345 = v123_345.as_joint();
    let j345_789 = v345_789.as_joint();

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
fn test_find_coherent_joint() -> anyhow::Result<()> {
    let s1 = || btreeset! {1,2,3};
    let s2 = || btreeset! {3,4,5};
    let s3 = || btreeset! {7,8,9};

    let j1 = Joint::from(vec![s1()]);
    let j2 = Joint::from(vec![s2()]);
    let j12 = Joint::from(vec![s1(), s2()]);
    let j23 = Joint::from(vec![s2(), s3()]);

    assert_eq!(j1, j1.find_coherent(s1()));
    assert_eq!(j12, j1.find_coherent(s2()));
    assert_eq!(j1, j12.find_coherent(s1()));
    assert_eq!(j2, j12.find_coherent(s2()));
    assert_eq!(j23, j12.find_coherent(s3()));

    Ok(())
}
