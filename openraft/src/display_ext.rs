//! Implement [`std::fmt::Display`] for types such as `Option<T>` and slice `&[T]`.

pub(crate) mod display_btreemap;
pub(crate) mod display_btreemap_debug_value;
pub(crate) mod display_btreemap_opt_value;
pub(crate) mod display_instant;
#[allow(unused_imports)]
pub(crate) use display_btreemap::DisplayBTreeMap;
#[allow(unused_imports)]
pub(crate) use display_btreemap::DisplayBTreeMapExt;
#[allow(unused_imports)]
pub(crate) use display_btreemap_debug_value::DisplayBTreeMapDebugValue;
#[allow(unused_imports)]
pub(crate) use display_btreemap_debug_value::DisplayBTreeMapDebugValueExt;
pub(crate) use display_btreemap_opt_value::DisplayBTreeMapOptValue;
#[allow(unused_imports)]
pub(crate) use display_btreemap_opt_value::DisplayBtreeMapOptValueExt;
#[allow(unused_imports)]
pub(crate) use display_instant::DisplayInstant;
pub(crate) use display_instant::DisplayInstantExt;
