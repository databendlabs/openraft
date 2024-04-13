use openraft_macros::expand;

#[allow(dead_code)]
#[allow(unused_variables)]
fn foo() {
    expand!(
        KEYED,
        // template with variables K and V
        (K, T, V) => {let K: T = V;},
        // arguments for rendering the template
        (a, u64, 1),
        (b, String, "foo".to_string()),
        (a, u32, 2), // duplicate `a` will be ignored
        (c, Vec<u8>, vec![1,2]),
    );

    let _x = 1;

    expand!(
        !KEYED,
        (K, T, V) => {let K: T = V;},
        (c, u8, 8),
        (c, u16, 16),
    );

    expand!(
        !KEYED,
        (K, M, T) => {M let K: T;},
        (c, , u8, ),
        (c, #[allow(dead_code)] , u16),
        (c, #[allow(dead_code)] #[allow(dead_code)] , u16),
    );
}

// #[parse_it]
// #[allow(dead_code)]
// fn bar() {}
