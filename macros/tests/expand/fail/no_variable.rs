fn main() {
    openraft_macros::expand!(
        KEYED,
        () => {},
    );

    openraft_macros::expand!(
        !KEYED,
        () => {},
    );

    openraft_macros::expand!(
        KEYED,
        () => {},
        (),
    );
}
