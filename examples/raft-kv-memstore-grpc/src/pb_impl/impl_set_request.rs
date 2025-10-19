use std::fmt;
use std::fmt::Formatter;

use crate::protobuf as pb;

impl fmt::Display for pb::SetRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "SetRequest {{ key: {}, value: {} }}", self.key, self.value)
    }
}
