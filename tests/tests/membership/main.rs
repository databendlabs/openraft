#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_learner_restart;
mod t10_single_node;
mod t11_add_learner;
mod t12_concurrent_write_and_add_learner;
mod t20_change_membership;
mod t21_change_membership_cases;
mod t30_commit_joint_config;
mod t30_elect_with_new_config;
mod t31_add_remove_follower;
mod t31_remove_leader;
mod t31_removed_follower;
mod t51_remove_unreachable_follower;
mod t99_issue_471_adding_learner_uses_uninit_leader_id;
mod t99_issue_584_replication_state_reverted;
mod t99_new_leader_auto_commit_uniform_config;
