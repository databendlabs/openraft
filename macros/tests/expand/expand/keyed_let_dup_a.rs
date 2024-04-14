fn main() {
    openraft_macros::expand!(
        KEYED,
        (K, T, V) => {let K: T = V;},
        (a, u64, 1),
        (b, String, "foo".to_string()),
        (a, u32, 2), // duplicate `a` will be ignored
    );
}
