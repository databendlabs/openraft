use std::fmt::Debug;

use crate::OptionalFeatures;

/// Application-defined metadata attached to a membership configuration as a whole.
///
/// `()` is the default for applications that do not need membership metadata.
pub trait MembershipMetadata
where
    Self: Sized + OptionalFeatures + Eq + Debug + Clone + Default + 'static,
{
}

impl<T> MembershipMetadata for T where T: Sized + OptionalFeatures + Eq + Debug + Clone + Default + 'static {}
