#![allow(unused_variables)]

fn main() {
    openraft_macros::expand!(
        KEYED,
        (K, T, V) => {let K: T = V;},
        (a, u64, 1),
    );
}
