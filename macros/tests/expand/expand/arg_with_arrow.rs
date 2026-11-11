fn g() -> u8 {
    1
}

fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, T, V) => {let K: T = V;},
        (f, fn() -> u8, g),
        (h, Box<dyn Fn() -> u8>, Box::new(|| 1)),
        (b, bool, 1 > 2,),
    );
}
