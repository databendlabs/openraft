use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

use crate::display_ext::DisplayInstantExt;
use crate::Instant;

/// A wrapper for [`Instant`] that supports serialization and deserialization.
///
/// This struct serializes an `Instant` into a string formatted as "%Y-%m-%dT%H:%M:%S%.9f%z", e.g.,
/// "2024-07-24T04:07:32.567025000+0000".
///
/// Note: Serialization and deserialization are not perfectly accurate and can be indeterministic,
/// resulting in minor variations each time. These deviations(could be smaller or greater) are
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
    pub fn new(inner: I) -> Self {
        Self { inner }
    }

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
    use serde::de;
    use serde::de::Visitor;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;

    use super::SerdeInstant;
    use crate::Instant;

    impl<I> SerdeInstant<I>
    where I: Instant
    {
        const SERDE_FMT: &'static str = "%Y-%m-%dT%H:%M:%S%.9f%z";
    }

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

            // Convert `SystemTime` to `DateTime<Utc>`
            let datetime: DateTime<Utc> = system_time.into();

            // Format the datetime to the desired string format
            let datetime_str = datetime.format(Self::SERDE_FMT).to_string();

            // Serialize the datetime string
            serializer.serialize_str(&datetime_str)
        }
    }

    impl<'de, I> Deserialize<'de> for SerdeInstant<I>
    where I: Instant
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de> {
            struct InstantVisitor<II: Instant>(PhantomData<II>);

            impl<'de, II: Instant> Visitor<'de> for InstantVisitor<II> {
                type Value = SerdeInstant<II>;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    // formatter.write_str("a string representing a datetime in the format %Y-%m-%dT%H:%M:%S%.6f%z")
                    write!(
                        formatter,
                        "a string representing a datetime in the format {}",
                        SerdeInstant::<II>::SERDE_FMT
                    )
                }

                fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                where E: de::Error {
                    // Parse the datetime string back to `DateTime<Utc>`
                    // 2024-07-19T09:30:46.635735000
                    let datetime =
                        DateTime::parse_from_str(value, SerdeInstant::<II>::SERDE_FMT).map_err(de::Error::custom)?;

                    // Convert `DateTime<Utc>` to `SystemTime`
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

            deserializer.deserialize_str(InstantVisitor::<I>(Default::default()))
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::SerdeInstant;
        use crate::engine::testing::UTConfig;
        use crate::type_config::alias::InstantOf;
        use crate::type_config::TypeConfigExt;

        #[test]
        fn test_serde_instant() {
            let now = UTConfig::<()>::now();
            let serde_instant = SerdeInstant::new(now);
            let json = serde_json::to_string(&serde_instant).unwrap();
            let deserialized: SerdeInstant<InstantOf<UTConfig>> = serde_json::from_str(&json).unwrap();

            println!("Now: {:?}", now);
            println!("Des: {:?}", *deserialized);
            // Convert Instant to SerdeInstant is inaccurate.
            if now > *deserialized {
                assert!((now - *deserialized) < Duration::from_millis(500));
            } else {
                assert!((*deserialized - now) < Duration::from_millis(500));
            }

            // Test serialization format

            let timestamp = r#""2024-07-24T04:07:32.567025000+0000""#;
            let deserialized: SerdeInstant<InstantOf<UTConfig>> = serde_json::from_str(&timestamp).unwrap();
            let serialized = serde_json::to_string(&deserialized).unwrap();

            assert_eq!(
                timestamp[0..24],
                serialized[0..24],
                "compare upto milli seconds: {}",
                timestamp[0..24]
            );
        }
    }
}
