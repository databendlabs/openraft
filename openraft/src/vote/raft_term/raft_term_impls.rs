use crate::vote::RaftTerm;

macro_rules! impl_raft_term {
    ($($t:ty),*) => {
        $(
            impl RaftTerm for $t {
                fn next(&self) -> Self {
                    self + 1
                }

                fn as_u64(&self) -> Option<u64> {
                    Some((*self).into())
                }
            }
        )*
    }
}

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

    #[test]
    fn test_as_u64() {
        assert_eq!(1_u8.as_u64(), Some(1));
        assert_eq!(255_u8.as_u64(), Some(255));
        assert_eq!(1_u16.as_u64(), Some(1));
        assert_eq!(1_u32.as_u64(), Some(1));
        assert_eq!(1_u64.as_u64(), Some(1));
        assert_eq!(u64::MAX.as_u64(), Some(u64::MAX));
    }
}
