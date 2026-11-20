/// Upgrade Self to type `To`.
///
/// This trait is used to define types that can be directly upgraded from older versions of openraft
/// to newer versions. For example, `LogId` can be upgraded: in openraft 0.7, `LogId` is `(term,
/// index)`, which is upgraded to `(CommittedLeaderId, index)` in openraft 0.8.
pub trait Upgrade<To> {
    /// Upgrades the current instance to type To.
    fn upgrade(self) -> To;

    /// Tries to upgrade to type `To`, and returns `self` with a reason when it cannot.
    ///
    /// The default calls [`upgrade()`](Self::upgrade), which cannot fail.
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

    fn try_upgrade(self) -> Result<To, (Self, &'static str)> {
        match self {
            Self::Old(o) => o.try_upgrade().map_err(|(old, reason)| (Self::Old(old), reason)),
            Self::New(x) => Ok(x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Compat;
    use super::Upgrade;

    #[derive(Debug)]
    struct Old(u8);

    impl Upgrade<u16> for Old {
        fn upgrade(self) -> u16 {
            self.0.into()
        }

        fn try_upgrade(self) -> Result<u16, (Self, &'static str)> {
            if self.0 == 0 {
                Err((self, "reserved value"))
            } else {
                Ok(self.upgrade())
            }
        }
    }

    #[test]
    fn try_upgrade() {
        tracing::info!(value = 7, "upgrade a supported old value");
        {
            let old = Compat::Old(Old(7));
            assert_eq!(old.try_upgrade().unwrap(), 7);
        }

        tracing::info!(value = 0, "return a rejected old value and its reason");
        {
            let old = Compat::Old(Old(0));
            let (input, reason) = old.try_upgrade().expect_err("reserved value must be rejected");
            assert!(matches!(input, Compat::Old(Old(0))));
            assert_eq!(reason, "reserved value");
        }

        tracing::info!(value = 300, "pass through an already-current value");
        {
            let new = Compat::<Old, _>::New(300);
            assert_eq!(new.try_upgrade().unwrap(), 300);
        }
    }
}
