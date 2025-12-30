use crate::vote::RaftTerm;

/// Implement RaftTerm for types that infallibly convert to u64.
macro_rules! impl_raft_term_infallible {
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

/// Implement RaftTerm for types that may fail to convert to u64.
macro_rules! impl_raft_term_fallible {
    ($($t:ty),*) => {
        $(
            impl RaftTerm for $t {
                fn next(&self) -> Self {
                    self + 1
                }

                fn as_u64(&self) -> Option<u64> {
                    (*self).try_into().ok()
                }
            }
        )*
    }
}

impl_raft_term_infallible!(u8, u16, u32, u64);
impl_raft_term_fallible!(u128, i8, i16, i32, i64, i128, usize, isize);

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
        assert_eq!(1_u128.next(), 2_u128);

        // Test signed integers
        assert_eq!(1_i8.next(), 2_i8);
        assert_eq!(1_i16.next(), 2_i16);
        assert_eq!(1_i32.next(), 2_i32);
        assert_eq!(1_i64.next(), 2_i64);
        assert_eq!(1_i128.next(), 2_i128);

        // Test default values
        assert_eq!(u8::default().next(), 1_u8);
        assert_eq!(u64::default().next(), 1_u64);
        assert_eq!(i64::default().next(), 1_i64);

        // Test boundary cases
        assert_eq!(254_u8.next(), 255_u8);
    }

    #[test]
    fn test_as_u64() {
        // Infallible conversions
        assert_eq!(1_u8.as_u64(), Some(1));
        assert_eq!(255_u8.as_u64(), Some(255));
        assert_eq!(1_u16.as_u64(), Some(1));
        assert_eq!(1_u32.as_u64(), Some(1));
        assert_eq!(1_u64.as_u64(), Some(1));
        assert_eq!(u64::MAX.as_u64(), Some(u64::MAX));

        // Fallible conversions - positive values
        assert_eq!(1_i8.as_u64(), Some(1));
        assert_eq!(1_i64.as_u64(), Some(1));
        assert_eq!(1_u128.as_u64(), Some(1));

        // Fallible conversions - negative values return None
        assert_eq!((-1_i8).as_u64(), None);
        assert_eq!((-1_i64).as_u64(), None);

        // Fallible conversions - overflow returns None
        assert_eq!((u64::MAX as u128 + 1).as_u64(), None);
    }
}
