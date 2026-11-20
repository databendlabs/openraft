#![allow(unused_variables)]

fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, M, V) => {M let K: u8 V;},
        (c, , ,),
        (c, , = 1 ),
    );
}
