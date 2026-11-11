fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, A) => {fn K() { A }},
        (a, #![allow(unused)]),
    );
}
