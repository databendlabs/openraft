extern crate test;

use maplit::btreeset;
use test::Bencher;
use test::black_box;

use crate::EffectiveMembership;
use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::quorum::QuorumSet;

#[bench]
fn m12345_ids_slice(b: &mut Bencher) {
    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = EffectiveMembership::new(None, m);
    let x = [1, 2, 3, 6, 7];

    b.iter(|| m.is_quorum(black_box(x.iter())))
}

#[bench]
fn m12345_ids_btreeset(b: &mut Bencher) {
    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = EffectiveMembership::new(None, m);
    let x = btreeset! {1, 2, 3, 6, 7};

    b.iter(|| m.is_quorum(black_box(x.iter())))
}

#[bench]
fn m12345_678_ids_slice(b: &mut Bencher) {
    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = EffectiveMembership::new(None, m);
    let x = [1, 2, 3, 6, 7];

    b.iter(|| m.is_quorum(black_box(x.iter())))
}

#[bench]
fn m12345_678_ids_btreeset(b: &mut Bencher) {
    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = EffectiveMembership::new(None, m);
    let x = btreeset! {1, 2, 3, 6, 7};

    b.iter(|| m.is_quorum(black_box(x.iter())))
}
