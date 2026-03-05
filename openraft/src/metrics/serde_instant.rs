use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

use crate::Instant;
use crate::display_ext::DisplayInstantExt;

/// A wrapper for [`Instant`] that supports serialization and deserialization.
///
/// This struct serializes an `Instant` into an `i64` which is the number of non-leap-nanoseconds
/// since January 1, 1970 UTC.
///
/// Note: Serialization and deserialization are not perfectly accurate and can be indeterministic,
/// resulting in minor variations each time. These deviations (could be smaller or greater) are
/// typically less than a microsecond (10^-6 seconds).
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
    use std::time::SystemTime;

    use chrono::DateTime;
    use chrono::Utc;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;
    use serde::de;
    use serde::de::Visitor;

    use super::SerdeInstant;
    use crate::Instant;

    impl<I> Serialize for SerdeInstant<I>
    where I: Instant
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer {
            // Convert Instant to SystemTime
            let system_time = {
                let sys_now = SystemTime::now();
                let now = I::now();

                if now >= self.inner {
                    let d = now - self.inner;
                    sys_now - d
                } else {
                    let d = self.inner - now;
                    sys_now + d
                }
            };

            let datetime: DateTime<Utc> = system_time.into();

            let nano = datetime.timestamp_nanos_opt().ok_or(serde::ser::Error::custom("time out of range"))?;

            serializer.serialize_u64(nano as u64)
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
                    write!(formatter, "an u64 generated with Datetime::timestamp_nanos_opt()",)
                }

                fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
                where E: de::Error {
                    let datetime = DateTime::from_timestamp_nanos(value as i64);

                    let system_time: SystemTime = datetime.with_timezone(&Utc).into();

                    // Calculate the `Instant` from the current time
                    let sys_now = SystemTime::now();
                    let now = II::now();
                    let instant = if system_time > sys_now {
                        now + (system_time.duration_since(sys_now).unwrap())
                    } else {
                        now - (sys_now.duration_since(system_time).unwrap())
                    };
                    Ok(SerdeInstant { inner: instant })
                }
            }

            deserializer.deserialize_u64(InstantVisitor::<I>(Default::default()))
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::SerdeInstant;
        use crate::engine::testing::UTConfig;
        use crate::type_config::TypeConfigExt;
        use crate::type_config::alias::SerdeInstantOf;

        #[test]
        fn test_serde_instant() {
            let now = UTConfig::<()>::now();
            let serde_instant = SerdeInstant::new(now);
            let json = serde_json::to_string(&serde_instant).unwrap();
            println!("json: {}", json);
            println!("Now: {:?}", now);

            let deserialized: SerdeInstantOf<UTConfig> = serde_json::from_str(&json).unwrap();
            println!("Des: {:?}", *deserialized);

            // Convert Instant to SerdeInstant is inaccurate.
            if now > *deserialized {
                assert!((now - *deserialized) < Duration::from_millis(5));
            } else {
                assert!((*deserialized - now) < Duration::from_millis(5));
            }

            // Test serialization format

            let nano = "1721829051211301916";
            let deserialized: SerdeInstantOf<UTConfig> = serde_json::from_str(nano).unwrap();
            let serialized = serde_json::to_string(&deserialized).unwrap();

            assert_eq!(
                nano[0..nano.len() - 6],
                serialized[0..serialized.len() - 6],
                "compare up to milli seconds: {}",
                &nano[0..nano.len() - 6]
            );
        }
    }
}

#[cfg(feature = "rkyv")]
mod rkyv_impl {
    use std::time::SystemTime;

    use chrono::DateTime;
    use chrono::Utc;

    use super::SerdeInstant;
    use crate::Instant;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, rkyv::Portable)]
    #[repr(transparent)]
    pub struct ArchivedSerdeInstant(pub rkyv::primitive::ArchivedU64);

    unsafe impl<C> rkyv::bytecheck::CheckBytes<C> for ArchivedSerdeInstant
    where
        C: rkyv::rancor::Fallible + ?Sized,
        rkyv::primitive::ArchivedU64: rkyv::bytecheck::CheckBytes<C>,
    {
        unsafe fn check_bytes(value: *const Self, context: &mut C) -> Result<(), C::Error> {
            // SAFETY: ArchivedSerdeInstant is repr(transparent) over ArchivedU64.
            unsafe {
                <rkyv::primitive::ArchivedU64 as rkyv::bytecheck::CheckBytes<C>>::check_bytes(
                    value.cast::<rkyv::primitive::ArchivedU64>(),
                    context,
                )
            }
        }
    }

    impl<I> rkyv::Archive for SerdeInstant<I>
    where I: Instant
    {
        type Archived = ArchivedSerdeInstant;
        type Resolver = u64;

        fn resolve(&self, resolver: Self::Resolver, out: rkyv::Place<Self::Archived>) {
            let archived = ArchivedSerdeInstant(rkyv::primitive::ArchivedU64::from_native(resolver));
            // SAFETY: ArchivedSerdeInstant is repr(transparent) over ArchivedU64 and has no padding.
            unsafe { out.write_unchecked(archived) };
        }
    }

    impl<I, S> rkyv::Serialize<S> for SerdeInstant<I>
    where
        I: Instant,
        S: rkyv::rancor::Fallible + ?Sized,
    {
        fn serialize(&self, _serializer: &mut S) -> Result<Self::Resolver, S::Error> {
            // Convert Instant to SystemTime.
            let system_time = {
                let sys_now = SystemTime::now();
                let now = I::now();

                if now >= self.inner {
                    let d = now - self.inner;
                    sys_now - d
                } else {
                    let d = self.inner - now;
                    sys_now + d
                }
            };

            let datetime: DateTime<Utc> = system_time.into();
            let nano = datetime.timestamp_nanos_opt().expect("time out of range");
            Ok(nano as u64)
        }
    }

    impl<I, D> rkyv::Deserialize<SerdeInstant<I>, D> for ArchivedSerdeInstant
    where
        I: Instant,
        D: rkyv::rancor::Fallible + ?Sized,
    {
        fn deserialize(&self, _deserializer: &mut D) -> Result<SerdeInstant<I>, D::Error> {
            let datetime = DateTime::from_timestamp_nanos(self.0.to_native() as i64);
            let system_time: SystemTime = datetime.with_timezone(&Utc).into();

            let sys_now = SystemTime::now();
            let now = I::now();
            let instant = if system_time > sys_now {
                now + system_time.duration_since(sys_now).unwrap()
            } else {
                now - sys_now.duration_since(system_time).unwrap()
            };

            Ok(SerdeInstant { inner: instant })
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::SerdeInstant;
        use crate::engine::testing::UTConfig;
        use crate::type_config::TypeConfigExt;
        use crate::type_config::alias::SerdeInstantOf;

        #[test]
        fn test_rkyv_instant() {
            let now = UTConfig::<()>::now();
            let serde_instant = SerdeInstant::new(now);

            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&serde_instant).unwrap();

            let deserialized: SerdeInstantOf<UTConfig> =
                rkyv::from_bytes::<SerdeInstantOf<UTConfig>, rkyv::rancor::Error>(&bytes).unwrap();

            // Convert Instant to SerdeInstant is inaccurate.
            if now > *deserialized {
                assert!((now - *deserialized) < Duration::from_millis(5));
            } else {
                assert!((*deserialized - now) < Duration::from_millis(5));
            }
        }
    }
}
