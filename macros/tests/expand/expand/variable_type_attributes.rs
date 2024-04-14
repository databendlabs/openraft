fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, M, T) => {M let K: T;},
        (c, , u8, ),
        (c, #[allow(dead_code)] , u16),
        (c, #[allow(dead_code)] #[allow(dead_code)] , u16),
    );
}
