#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t00_learner_restart;
mod t10_add_learner;
mod t20_change_membership;
mod t25_elect_with_new_config;
mod t30_commit_joint_config;
mod t40_removed_follower;
mod t99_new_leader_auto_commit_uniform_config;
