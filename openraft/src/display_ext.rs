//! Implement [`fmt::Display`] for types such as `Option<T>` and slice `&[T]`.

pub(crate) mod display_instant;
pub(crate) mod display_option;
pub(crate) mod display_slice;
