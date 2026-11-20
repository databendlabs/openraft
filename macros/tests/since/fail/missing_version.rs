#[openraft_macros::since(date = "2021-01-01")]
fn date_only() {}

// A bare `#[since]` only enables field markers on a struct or enum; on an item it needs a version.
#[openraft_macros::since]
fn bare() {}

fn main() {}
