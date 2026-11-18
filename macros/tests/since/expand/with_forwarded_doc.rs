macro_rules! forward_doc {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[openraft_macros::since(version = "1.0.0")]
        const $name: i32 = 0;
    };
}

forward_doc! {
    /// Doc
    A
}
