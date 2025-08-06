/// Upgrade Self to type `To`.
///
/// This trait is used to define types that can be directly upgraded from older versions of openraft
/// to newer versions. For example, `LogId` can be upgraded: in openraft 0.7, `LogId` is `(term,
/// index)`, which is upgraded to `(CommittedLeaderId, index)` in openraft 0.8.
pub trait Upgrade<To> {
    /// Upgrades the current instance to type To.
    fn upgrade(self) -> To;

    fn try_upgrade(self) -> Result<To, (Self, &'static str)>
    where Self: Sized {
        Ok(self.upgrade())
    }
}

/// `Compat` is a serialization-compatible type that can be deserialized from both an older type
/// and a newer type. It serves as an intermediate type container for newer programs to read old
/// data.
#[derive(Debug)]
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Compat<From, To>
where From: Upgrade<To>
{
    /// Represents the older version of the data.
    Old(From),
    /// Represents the newer version of the data.
    New(To),
}

/// A compatible type can be upgraded to `To` if `From` can be upgraded to `To`.
impl<From, To> Upgrade<To> for Compat<From, To>
where From: Upgrade<To>
{
    fn upgrade(self) -> To {
        match self {
            Self::Old(o) => o.upgrade(),
            Self::New(x) => x,
        }
    }
}
