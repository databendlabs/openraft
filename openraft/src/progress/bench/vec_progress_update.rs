extern crate test;

use test::black_box;
use test::Bencher;

use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::Joint;

#[bench]
fn progress_update_01234_567(b: &mut Bencher) {
    let membership: Vec<Vec<u64>> = vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7]];
    let quorum_set = Joint::from(membership);
    let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, 0..=7, 0);

    let mut id = 0u64;
    let mut values = [0, 1, 2, 3, 4, 5, 6, 7];
    b.iter(|| {
        id = (id + 1) & 7;
        values[id as usize] += 1;
        let v = values[id as usize];

        let _ = progress.update(&black_box(id), black_box(v));
    });

    // It shows that is_quorum() is called at a rate of about 1/4 of update()
    // `Stat { update_count: 42997501, move_count: 10749381, is_quorum_count: 10749399 }`
    // println!("progress stat: {:?}", progress.stat());
}
