//! Implement [`std::fmt::Display`] for types such as `Option<T>` and slice `&[T]`.

pub(crate) mod display_instant;
pub(crate) mod display_option;
pub(crate) mod display_result;
pub(crate) mod display_slice;

#[allow(unused_imports)]
pub(crate) use display_instant::DisplayInstant;
pub(crate) use display_instant::DisplayInstantExt;
pub(crate) use display_option::DisplayOption;
pub(crate) use display_option::DisplayOptionExt;
#[allow(unused_imports)]
pub(crate) use display_result::DisplayResult;
pub(crate) use display_result::DisplayResultExt;
pub(crate) use display_slice::DisplaySlice;
pub(crate) use display_slice::DisplaySliceExt;
