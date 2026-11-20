#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/expand/fail/*.rs");
}

#[test]
fn pass() {
    // Each input needs a checked-in `*.expanded.rs`. To write missing snapshots and replace
    // changed ones, run this test with `MACROTEST=overwrite`.
    if std::env::var_os("MACROTEST").is_some_and(|v| v == "overwrite") {
        macrotest::expand("tests/expand/expand/*.rs");
    } else {
        macrotest::expand_without_refresh("tests/expand/expand/*.rs");
    }
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
