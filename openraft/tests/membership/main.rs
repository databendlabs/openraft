#![cfg_attr(feature = "bt", feature(backtrace))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t00_learner_restart;
mod t01_single_node;
mod t10_add_learner;
mod t12_concurrent_write_and_add_learner;
mod t15_add_remove_follower;
mod t16_change_membership_cases;
mod t20_change_membership;
mod t25_elect_with_new_config;
mod t30_commit_joint_config;
mod t30_step_down;
mod t40_removed_follower;
mod t45_remove_unreachable_follower;
mod t99_issue_471_adding_learner_uses_uninit_leader_id;
mod t99_new_leader_auto_commit_uniform_config;
