fn main() {
    openraft_macros::expand!(
        !KEYED,
        (K, V) => {let K = (V, V); let _ = [K]; { let _ = K; }},
        (a, 1),
    );
}
