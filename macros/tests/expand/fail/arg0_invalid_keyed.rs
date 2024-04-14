fn main() {
    openraft_macros::expand!(
        !FOO,
        (K, T, V) => {K; T; V;},
    );

    openraft_macros::expand!(
        FOO,
        (K, T, V) => {K; T; V;},
    );
}
