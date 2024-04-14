#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/since/fail/*.rs");
}

#[test]
fn pass() {
    macrotest::expand("tests/since/expand/*.rs");
}
