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

impl_raft_term!(
    u8, u16, u32, u64, u128, //
    i8, i16, i32, i64, i128, //
    usize, isize
);

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
        assert_eq!(1_usize.next(), 2_usize);

        // Test signed integers
        assert_eq!(1_i8.next(), 2_i8);
        assert_eq!(1_i16.next(), 2_i16);
        assert_eq!(1_i32.next(), 2_i32);
        assert_eq!(1_i64.next(), 2_i64);
        assert_eq!(1_i128.next(), 2_i128);
        assert_eq!(1_isize.next(), 2_isize);

        // Test default values
        assert_eq!(u8::default().next(), 1_u8);
        assert_eq!(i8::default().next(), 1_i8);
        assert_eq!(usize::default().next(), 1_usize);
        assert_eq!(isize::default().next(), 1_isize);

        // Test boundary cases
        assert_eq!(254_u8.next(), 255_u8);
        assert_eq!(126_i8.next(), 127_i8);
        assert_eq!((-2_i8).next(), -1_i8);
    }
}
