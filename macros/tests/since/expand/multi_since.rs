#[openraft_macros::since(
    version = "1.1.0",
    date = "2021-02-01",
    change = "add Foo",
    change = "add Bar",
    change = "add Baz"
)]
#[openraft_macros::since(version = "1.0.0", date = "2021-01-01", change = "Added")]
const A: i32 = 0;
