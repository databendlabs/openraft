use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

#[derive(Debug, Clone, PartialEq)]
struct NodeId {
    field_1: u32,
    field_2: u64,
}

impl Serialize for NodeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        let string_repr = format!("{}:{}", self.field_1, self.field_2);
        serializer.serialize_str(&string_repr)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        let parts: Vec<&str> = s.split(':').collect();

        if parts.len() != 2 {
            return Err(serde::de::Error::custom(
                "NodeId string must be in format 'field_1:field_2'",
            ));
        }

        let field_1 = parts[0].parse::<u32>().map_err(|_| serde::de::Error::custom("Failed to parse field_1"))?;
        let field_2 = parts[1].parse::<u64>().map_err(|_| serde::de::Error::custom("Failed to parse field_2"))?;

        Ok(NodeId { field_1, field_2 })
    }
}

#[cfg(test)]
mod tests {
    use serde_json;

    use super::*;

    #[test]
    fn test_serialize_node_id() {
        let node_id = NodeId { field_1: 1, field_2: 2 };

        let serialized = serde_json::to_string(&node_id).unwrap();
        println!("serialized: {}", serialized);
        assert_eq!(serialized, "\"1:2\"");
    }

    #[test]
    fn test_deserialize_node_id() {
        let json_str = "\"1:2\"";
        let node_id: NodeId = serde_json::from_str(json_str).unwrap();

        assert_eq!(node_id.field_1, 1);
        assert_eq!(node_id.field_2, 2);
    }

    #[test]
    fn test_deserialize_invalid_format() {
        let invalid_json = "\"invalid_format\"";
        let result: Result<NodeId, _> = serde_json::from_str(invalid_json);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("NodeId string must be in format 'field_1:field_2'"));
    }

    #[test]
    fn test_deserialize_invalid_number() {
        let invalid_json = "\"1:not_a_number\"";
        let result: Result<NodeId, _> = serde_json::from_str(invalid_json);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Failed to parse field_2"));
    }
}
