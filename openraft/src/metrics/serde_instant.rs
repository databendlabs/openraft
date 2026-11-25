use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

use openraft_macros::since;

use crate::Instant;
use crate::display_ext::DisplayInstantExt;

/// A wrapper for [`Instant`] that supports serialization and deserialization.
///
/// With the `serde` feature of `openraft`, `SerdeInstant` implements `Serialize` and
/// `Deserialize`. It encodes an instant as its wall-clock time: a `u64` in the serde data model,
/// the number of nanoseconds since the Unix epoch, 1970-01-01 00:00:00 UTC. Deserialization
/// accepts only an unsigned integer, so a format that delivers the value as a signed integer
/// fails with a type error.
///
/// The value ranges from 0, the Unix epoch, to `i64::MAX`, 2262-04-11 23:47:16.854775807 UTC,
/// so that it also fits an `i64`. Serialization and deserialization return an error for a time
/// outside this range. Deserialization also returns an error for a time that the runtime's clock
/// cannot represent, such as a time before a simulated clock started.
///
/// The conversion is approximate. Each serialization or deserialization reads the system clock
/// and the runtime's clock one after the other, and maps the instant through the offset between
/// them. An immediate round trip on one machine typically shifts an instant by less than a
/// microsecond. The shift is larger when the offset changes between the two conversions, such as
/// when the system clock is adjusted, or when the runtime's clock does not follow real time. A
/// value decoded on another machine also differs by the offset between the two machines' system
/// clocks.
#[since(
    version = "0.10.2",
    change = "deserialization rejects a time past `i64::MAX` nanoseconds, as serialization does"
)]
#[since(
    version = "0.10.2",
    change = "a time out of a clock's range returns a serde error instead of panicking"
)]
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
pub struct SerdeInstant<I>
where I: Instant
{
    inner: I,
}

impl<I> Deref for SerdeInstant<I>
where I: Instant
{
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> From<I> for SerdeInstant<I>
where I: Instant
{
    fn from(inner: I) -> Self {
        Self { inner }
    }
}

impl<I> fmt::Display for SerdeInstant<I>
where I: Instant
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.inner.display().fmt(f)
    }
}

impl<I> SerdeInstant<I>
where I: Instant
{
    /// Create a new SerdeInstant wrapping the given Instant.
    pub fn new(inner: I) -> Self {
        Self { inner }
    }

    /// Extract the inner Instant value.
    pub fn into_inner(self) -> I {
        self.inner
    }
}

#[cfg(feature = "serde")]
mod serde_impl {
    use std::fmt;
    use std::marker::PhantomData;
    use std::time::Duration;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;
    use serde::de;
    use serde::de::Visitor;

    use super::SerdeInstant;
    use crate::Instant;

    /// The latest time the encoding represents, in nanoseconds since the Unix epoch: `i64::MAX`,
    /// in year 2262, so that the value also fits an `i64`. Serialization and deserialization
    /// both reject a later time.
    const MAX_NANOS: u64 = i64::MAX as u64;

    impl<I> Serialize for SerdeInstant<I>
    where I: Instant
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer {
            // Convert Instant to SystemTime, or `None` if `SystemTime` cannot represent it.
            let system_time = {
                let sys_now = SystemTime::now();
                let now = I::now();

                if now >= self.inner {
                    let d = now - self.inner;
                    sys_now.checked_sub(d)
                } else {
                    let d = self.inner - now;
                    sys_now.checked_add(d)
                }
            };

            // Nanoseconds since the Unix epoch. Reject times before the epoch or past `MAX_NANOS`.
            let nano = system_time
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| u64::try_from(d.as_nanos()).ok())
                .filter(|nano| *nano <= MAX_NANOS)
                .ok_or_else(|| serde::ser::Error::custom("time out of range"))?;

            serializer.serialize_u64(nano)
        }
    }

    impl<'de, I> Deserialize<'de> for SerdeInstant<I>
    where I: Instant
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de> {
            struct InstantVisitor<II: Instant>(PhantomData<II>);

            impl<II: Instant> Visitor<'_> for InstantVisitor<II> {
                type Value = SerdeInstant<II>;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    write!(formatter, "a u64 count of nanoseconds since the Unix epoch")
                }

                fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
                where E: de::Error {
                    // Reject a time past `MAX_NANOS`, as serialization does, even when both clocks
                    // can represent it.
                    if value > MAX_NANOS {
                        return Err(E::custom("time out of range"));
                    }

                    let system_time = UNIX_EPOCH
                        .checked_add(Duration::from_nanos(value))
                        .ok_or_else(|| E::custom("time out of range"))?;

                    // Calculate the `Instant` from the current time. The runtime's clock may not
                    // represent it, such as a time before a simulated clock started.
                    let sys_now = SystemTime::now();
                    let now = II::now();
                    let instant = if system_time > sys_now {
                        now.checked_add(system_time.duration_since(sys_now).unwrap())
                    } else {
                        now.checked_sub(sys_now.duration_since(system_time).unwrap())
                    };
                    let inner = instant.ok_or_else(|| E::custom("time out of range of the runtime's clock"))?;
                    Ok(SerdeInstant { inner })
                }
            }

            deserializer.deserialize_u64(InstantVisitor::<I>(Default::default()))
        }
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_instant_test;
