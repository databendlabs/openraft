//! RaftLogId implementation for primitive tuples `(Term, u64)`.

use crate::RaftTypeConfig;
use crate::log_id::raft_log_id::RaftLogId;
use crate::type_config::alias::CommittedLeaderIdOf;

macro_rules! impl_raft_log_id {
    ($term_type:ty) => {
        impl<C> RaftLogId<C> for ($term_type, u64)
        where C: RaftTypeConfig<Term = $term_type, LeaderId = crate::vote::leader_id_std::LeaderId<C>>
        {
            fn new(leader_id: CommittedLeaderIdOf<C>, index: u64) -> Self {
                (*leader_id, index)
            }

            fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
                // SAFETY: CommittedLeaderId<C> is repr(transparent) around C::Term.
                unsafe { &*(std::ptr::addr_of!(self.0) as *const CommittedLeaderIdOf<C>) }
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
    use crate::declare_raft_types;
    use crate::log_id::raft_log_id::RaftLogId;
    use crate::vote::leader_id_std::CommittedLeaderId;

    declare_raft_types!(pub ConfigU64: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=u64);
    declare_raft_types!(pub ConfigU32: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=u32);
    declare_raft_types!(pub ConfigU16: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=u16);
    declare_raft_types!(pub ConfigU8:  LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=u8);
    declare_raft_types!(pub ConfigI64: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=i64);
    declare_raft_types!(pub ConfigI32: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=i32);
    declare_raft_types!(pub ConfigI16: LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=i16);
    declare_raft_types!(pub ConfigI8:  LeaderId=crate::vote::leader_id_std::LeaderId<Self>, Term=i8);

    #[test]
    fn test_u64_tuple_log_id() {
        let log_id: (u64, u64) = (5, 100);
        assert_eq!(100, <(u64, u64) as RaftLogId<ConfigU64>>::index(&log_id));
        assert_eq!(5, **<(u64, u64) as RaftLogId<ConfigU64>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u64_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigU64>::new(5);
        let log_id = <(u64, u64) as RaftLogId<ConfigU64>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(u64, u64) as RaftLogId<ConfigU64>>::index(&log_id));
    }

    #[test]
    fn test_u32_tuple_log_id() {
        let log_id: (u32, u64) = (5, 100);
        assert_eq!(100, <(u32, u64) as RaftLogId<ConfigU32>>::index(&log_id));
        assert_eq!(5, **<(u32, u64) as RaftLogId<ConfigU32>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u32_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigU32>::new(5);
        let log_id = <(u32, u64) as RaftLogId<ConfigU32>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(u32, u64) as RaftLogId<ConfigU32>>::index(&log_id));
    }

    #[test]
    fn test_u16_tuple_log_id() {
        let log_id: (u16, u64) = (5, 100);
        assert_eq!(100, <(u16, u64) as RaftLogId<ConfigU16>>::index(&log_id));
        assert_eq!(5, **<(u16, u64) as RaftLogId<ConfigU16>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u16_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigU16>::new(5);
        let log_id = <(u16, u64) as RaftLogId<ConfigU16>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(u16, u64) as RaftLogId<ConfigU16>>::index(&log_id));
    }

    #[test]
    fn test_u8_tuple_log_id() {
        let log_id: (u8, u64) = (5, 100);
        assert_eq!(100, <(u8, u64) as RaftLogId<ConfigU8>>::index(&log_id));
        assert_eq!(5, **<(u8, u64) as RaftLogId<ConfigU8>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_u8_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigU8>::new(5);
        let log_id = <(u8, u64) as RaftLogId<ConfigU8>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(u8, u64) as RaftLogId<ConfigU8>>::index(&log_id));
    }

    #[test]
    fn test_i64_tuple_log_id() {
        let log_id: (i64, u64) = (5, 100);
        assert_eq!(100, <(i64, u64) as RaftLogId<ConfigI64>>::index(&log_id));
        assert_eq!(5, **<(i64, u64) as RaftLogId<ConfigI64>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i64_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigI64>::new(5);
        let log_id = <(i64, u64) as RaftLogId<ConfigI64>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(i64, u64) as RaftLogId<ConfigI64>>::index(&log_id));
    }

    #[test]
    fn test_i32_tuple_log_id() {
        let log_id: (i32, u64) = (5, 100);
        assert_eq!(100, <(i32, u64) as RaftLogId<ConfigI32>>::index(&log_id));
        assert_eq!(5, **<(i32, u64) as RaftLogId<ConfigI32>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i32_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigI32>::new(5);
        let log_id = <(i32, u64) as RaftLogId<ConfigI32>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(i32, u64) as RaftLogId<ConfigI32>>::index(&log_id));
    }

    #[test]
    fn test_i16_tuple_log_id() {
        let log_id: (i16, u64) = (5, 100);
        assert_eq!(100, <(i16, u64) as RaftLogId<ConfigI16>>::index(&log_id));
        assert_eq!(5, **<(i16, u64) as RaftLogId<ConfigI16>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i16_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigI16>::new(5);
        let log_id = <(i16, u64) as RaftLogId<ConfigI16>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(i16, u64) as RaftLogId<ConfigI16>>::index(&log_id));
    }

    #[test]
    fn test_i8_tuple_log_id() {
        let log_id: (i8, u64) = (5, 100);
        assert_eq!(100, <(i8, u64) as RaftLogId<ConfigI8>>::index(&log_id));
        assert_eq!(5, **<(i8, u64) as RaftLogId<ConfigI8>>::committed_leader_id(&log_id));
    }

    #[test]
    fn test_i8_tuple_log_id_new() {
        let leader_id = CommittedLeaderId::<ConfigI8>::new(5);
        let log_id = <(i8, u64) as RaftLogId<ConfigI8>>::new(leader_id, 100);
        assert_eq!((5, 100), log_id);
        assert_eq!(100, <(i8, u64) as RaftLogId<ConfigI8>>::index(&log_id));
    }
}
