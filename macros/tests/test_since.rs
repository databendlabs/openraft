#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/since/fail/*.rs");
}

#[test]
fn pass() {
    // Each input needs a checked-in `*.expanded.rs`. To write missing snapshots and replace
    // changed ones, run this test with `MACROTEST=overwrite`.
    if std::env::var_os("MACROTEST").is_some_and(|v| v == "overwrite") {
        macrotest::expand("tests/since/expand/*.rs");
    } else {
        macrotest::expand_without_refresh("tests/since/expand/*.rs");
    }
}
