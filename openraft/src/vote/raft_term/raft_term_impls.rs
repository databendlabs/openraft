use crate::vote::RaftTerm;

macro_rules! impl_raft_term {
    ($($t:ty),*) => {
        $(
            impl RaftTerm for $t {
                fn next(&self) -> Self {
                    self + 1
                }
            }
        )*
    }
}

// Only unsigned integers up to u64 are supported as RaftTerm
// because MetricsRecorder requires conversion to u64.
impl_raft_term!(u8, u16, u32, u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_term_impls() {
        // Test unsigned integers
        assert_eq!(1_u8.next(), 2_u8);
        assert_eq!(1_u16.next(), 2_u16);
        assert_eq!(1_u32.next(), 2_u32);
        assert_eq!(1_u64.next(), 2_u64);

        // Test default values
        assert_eq!(u8::default().next(), 1_u8);
        assert_eq!(u64::default().next(), 1_u64);

        // Test boundary cases
        assert_eq!(254_u8.next(), 255_u8);
    }
}
