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
