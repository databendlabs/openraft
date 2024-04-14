#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/expand/fail/*.rs");
}

#[test]
fn pass() {
    macrotest::expand("tests/expand/expand/*.rs");
}
