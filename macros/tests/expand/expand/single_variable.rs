#![allow(unused_variables)]

fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K,) => {let K: u8 = 1;},
        (a),
        (b,),
    );
}
