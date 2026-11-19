#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/add_async_trait/fail/*.rs");
}
