//! Application requests for the optional KV profile.

/// One logical write, whose ID is preserved across retries.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[openraft::macros::since(version = "0.10.1")]
pub struct ProfileRequest {
    pub operation: u64,
    pub key: u64,
    pub value: Vec<u8>,
}

impl ProfileRequest {
    /// Execute deterministic CPU work over the actual request data.
    pub(crate) fn work(&self, rounds: u64) {
        let mut state = self.operation;
        for _ in 0..rounds {
            state ^= self.key;
            state = state.rotate_left(13).wrapping_mul(0x9e3779b97f4a7c15);
            for byte in &self.value {
                state ^= u64::from(*byte);
                state = state.rotate_left(7).wrapping_mul(0xbf58476d1ce4e5b9);
            }
        }
        std::hint::black_box(state);
    }
}
