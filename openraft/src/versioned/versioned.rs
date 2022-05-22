use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::Arc;

use crate::MessageSummary;

/// Track data change by version.
///
/// For variable with interior mutability, such as those contain `Arc<Mutex<_>>` or `AtomicU64`,
/// a `version` is used to track changes.
///
/// Every update that is made to it increments the `version` by 1.
/// The inner `data` is an `Arc` thus `Clone` would be cheap enough.
///
/// **Caveat**: an instance will see changes made on another clone, since they reference the same data, until an
/// update-by-replace is made.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Versioned<Data>
where Data: Clone
{
    /// A version number that indicates every change to `data`.
    version: u64,

    /// The data.
    ///
    /// It may change in place(for `AtomicU64` fields) or be replace as a whole.
    /// The `version` should be incremented in any case.
    data: Arc<Data>,
}

impl<Data> Versioned<Data>
where Data: Clone
{
    pub fn new(d: Data) -> Self {
        Self {
            version: 0,
            data: Arc::new(d),
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn data(&self) -> &Arc<Data> {
        &self.data
    }
}

impl<Data> PartialEq for Versioned<Data>
where Data: Clone
{
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) && self.version == other.version
    }
}

impl<Data> Eq for Versioned<Data> where Data: Clone {}

impl<Data> Debug for Versioned<Data>
where Data: Clone + Debug
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Versioned").field("version", &self.version).field("data", &self.data).finish()
    }
}

impl<Data> MessageSummary for Versioned<Data>
where Data: Clone + MessageSummary
{
    fn summary(&self) -> String {
        format!("{{ver:{}, {}}}", self.version, self.data.summary())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("can not update in place")]
    CanNotUpdateInPlace,
}

/// Defines an update operation that can be applied to `Versioned<Data>`
///
/// The update try to update `Versioned.data` in place if possible.
/// If in place update can not be done, it then update it by replacing `Versioned.data` with a new cloned instance.
///
/// A user needs to implement two update methods: `update_in_place()` and `update_mut()`.
pub trait Update<Data>
where Data: Clone
{
    /// Try to apply an update `Versioned.data` in place if possible and return.
    /// Or replace `Versioned.data` with a new one.
    /// Finally it increments the `Versioned.version` by 1 to indicate the data has changed.
    fn apply(&self, to: &mut Versioned<Data>) {
        let res = self.apply_in_place(&to.data);

        if res.is_err() {
            let x = Arc::make_mut(&mut to.data);
            self.apply_mut(x);
        }

        to.version += 1;
    }

    /// Apply the update to the `Versioned.data` in place if possible.
    ///
    /// If it can not be done, it should return an error to inform it to update the `Versioned.data` by replacing it.
    fn apply_in_place(&self, to: &Arc<Data>) -> Result<(), UpdateError>;

    /// Apply the update a cloned new instance of `Versioned.data`.
    ///
    /// After updating it, `to` will replace the `data` in [`Versioned`].
    fn apply_mut(&self, to: &mut Data);
}

/// An object that can be updated by calling `self.update(arg)`
pub trait Updatable<U> {
    fn update(&mut self, arg: U);
}

/// If `U` is an `Update`, i.e. `U.apply(Versioned<Data>)`,
/// then install a method `update()` to `Versioned<Data>`: `Versioned<Data>.update(U)`.
impl<Data, U> Updatable<U> for Versioned<Data>
where
    Data: Clone,
    U: Update<Data>,
{
    fn update(&mut self, update: U) {
        update.apply(self)
    }
}
