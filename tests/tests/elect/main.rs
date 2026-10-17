#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

use tests::fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_elect_compare_last_log;
mod t11_elect_seize_leadership;
mod t12_pre_vote;
mod t13_elect_while_leader;
mod t14_one_way_isolation;
mod t15_lease_gates;
mod t16_disjoint_read_quorums;
mod t17_leader_alone_is_write_quorum;
mod t18_campaigner_outside_voters;
mod t60_quorum_loss_probe_interval;
