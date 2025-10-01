extern crate test;

use maplit::btreeset;
use test::Bencher;
use test::black_box;

use crate::quorum::AsJoint;
use crate::quorum::QuorumSet;

#[bench]
fn quorum_set_slice_ids_slice(b: &mut Bencher) {
    let m12345: &[usize] = &[1, 2, 3, 4, 5];
    let x = [1, 2, 3, 6, 7];
    b.iter(|| m12345.is_quorum(black_box(x.iter())))
}

#[bench]
fn quorum_set_vec_ids_slice(b: &mut Bencher) {
    let m12345 = vec![1, 2, 3, 4, 5];
    let x = [1, 2, 3, 6, 7];
    b.iter(|| m12345.is_quorum(black_box(x.iter())))
}

#[bench]
fn quorum_set_btreeset_ids_slice(b: &mut Bencher) {
    let m12345678 = btreeset! {1,2,3,4,5,6,7,8};
    let x = [1, 2, 3, 6, 7];
    b.iter(|| m12345678.is_quorum(black_box(x.iter())))
}

#[bench]
fn quorum_set_vec_of_vec_ids_slice(b: &mut Bencher) {
    let m12345_678 = vec![vec![1, 2, 3, 4, 5], vec![6, 7, 8]];
    let x = [1, 2, 3, 6, 7];
    b.iter(|| m12345_678.as_joint().is_quorum(black_box(x.iter())))
}

#[bench]
fn quorum_set_vec_of_btreeset_ids_slice(b: &mut Bencher) {
    let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];
    let x = [1, 2, 3, 6, 7];
    b.iter(|| m12345_678.as_joint().is_quorum(black_box(x.iter())))
}

#[bench]
fn quorum_set_vec_of_btreeset_ids_btreeset(b: &mut Bencher) {
    let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];
    let x = btreeset! {1,2,3,6,7};
    b.iter(|| m12345_678.as_joint().is_quorum(black_box(x.iter())))
}
