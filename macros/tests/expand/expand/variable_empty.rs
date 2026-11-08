fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, T, V) => {let K: Option<T V> = None;},
        (c, , ,),
        (c, , u8 ),
    );
}
