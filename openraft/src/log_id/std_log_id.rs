//! RaftLogId implementation for primitive tuples `(Term, u64)`.

use crate::log_id::raft_log_id::RaftLogId;
use crate::vote::leader_id_std;

macro_rules! impl_raft_log_id {
    ($term_type:ty) => {
        impl RaftLogId for ($term_type, u64) {
            type CommittedLeaderId = leader_id_std::CommittedLeaderId<$term_type>;

            fn new(leader_id: Self::CommittedLeaderId, index: u64) -> Self {
                (*leader_id, index)
            }

            fn committed_leader_id(&self) -> &Self::CommittedLeaderId {
                // SAFETY: CommittedLeaderId<Term> is repr(transparent) around Term.
                unsafe { &*(std::ptr::addr_of!(self.0) as *const Self::CommittedLeaderId) }
            }

            fn index(&self) -> u64 {
                self.1
            }
        }
    };
}

impl_raft_log_id!(u64);
impl_raft_log_id!(u32);
impl_raft_log_id!(u16);
impl_raft_log_id!(u8);
impl_raft_log_id!(i64);
impl_raft_log_id!(i32);
impl_raft_log_id!(i16);
impl_raft_log_id!(i8);

#[cfg(test)]
mod tests {
    use crate::log_id::raft_log_id::RaftLogId;
    use crate::vote::leader_id_std;

    type CLID64 = leader_id_std::CommittedLeaderId<u64>;
    type CLID32 = leader_id_std::CommittedLeaderId<u32>;
    type CLID16 = leader_id_std::CommittedLeaderId<u16>;
    type CLID8 = leader_id_std::CommittedLeaderId<u8>;
    type CLIDI64 = leader_id_std::CommittedLeaderId<i64>;
    type CLIDI32 = leader_id_std::CommittedLeaderId<i32>;
    type CLIDI16 = leader_id_std::CommittedLeaderId<i16>;
    type CLIDI8 = leader_id_std::CommittedLeaderId<i8>;

    #[test]
    fn test_u64_tuple_log_id() {
        let log_id: (u64, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u64_tuple_log_id_new() {
        let leader_id = CLID64::new(5);
        let log_id = <(u64, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_u32_tuple_log_id() {
        let log_id: (u32, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u32_tuple_log_id_new() {
        let leader_id = CLID32::new(5);
        let log_id = <(u32, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_u16_tuple_log_id() {
        let log_id: (u16, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u16_tuple_log_id_new() {
        let leader_id = CLID16::new(5);
        let log_id = <(u16, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_u8_tuple_log_id() {
        let log_id: (u8, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u8_tuple_log_id_new() {
        let leader_id = CLID8::new(5);
        let log_id = <(u8, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_i64_tuple_log_id() {
        let log_id: (i64, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i64_tuple_log_id_new() {
        let leader_id = CLIDI64::new(5);
        let log_id = <(i64, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_i32_tuple_log_id() {
        let log_id: (i32, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i32_tuple_log_id_new() {
        let leader_id = CLIDI32::new(5);
        let log_id = <(i32, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_i16_tuple_log_id() {
        let log_id: (i16, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i16_tuple_log_id_new() {
        let leader_id = CLIDI16::new(5);
        let log_id = <(i16, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }

    #[test]
    fn test_i8_tuple_log_id() {
        let log_id: (i8, u64) = (5, 100);
        assert_eq!(100, RaftLogId::index(&log_id));
        assert_eq!(5, **RaftLogId::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i8_tuple_log_id_new() {
        let leader_id = CLIDI8::new(5);
        let log_id = <(i8, u64) as RaftLogId>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, RaftLogId::index(&log_id));
    }
}
