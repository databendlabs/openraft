#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/expand/fail/*.rs");
}

#[test]
fn pass() {
    macrotest::expand("tests/expand/expand/*.rs");
}

/// Compile and run each input of `pass`: a matching snapshot does not prove that the expansion
/// type-checks.
#[test]
fn pass_compiles() {
    // `*.expanded.rs` files are the expected output, not inputs.
    let mut inputs: Vec<_> = std::fs::read_dir("tests/expand/expand")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let path = path.to_str().unwrap();
            path.ends_with(".rs") && !path.ends_with(".expanded.rs")
        })
        .collect();
    inputs.sort();

    let t = trybuild::TestCases::new();
    for input in inputs {
        t.pass(input);
    }
}
