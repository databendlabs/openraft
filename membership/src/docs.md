# Provided memberships: pick how each config states its quorums

Every membership in this crate gives each config read quorums and write
quorums. An entry commits once a write quorum stores it, and a candidate
becomes leader only with votes from a read quorum. Every read quorum shares a
voter with every write quorum, so a new leader holds every committed entry.
The memberships differ in how a config states its quorums. That decides how
many voters a commit and an election wait for, and which failures stop the
cluster:

| Membership                 | A config sets                                                                     | Pick it when                                                                                             |
|----------------------------|-----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| [`ThresholdMembership`]    | the voters and a write size                                                       | every voter counts the same, and you choose how many copies a commit needs                               |
| [`WeightedMembership`]     | a weight for each voter and a write weight                                        | some voters, or the sites that hold them, must count more                                                |
| [`ExplicitMembership`]     | the minimal read quorums and the minimal write quorums                            | no rule above states the policy, such as a backup voter in every write quorum and in no read quorum      |

[`ThresholdMembership`]: crate::ThresholdMembership
[`WeightedMembership`]: crate::WeightedMembership
[`ExplicitMembership`]: crate::ExplicitMembership
[`WeightedMembership::with_roles()`]: crate::WeightedMembership::with_roles
[`threshold`]: crate::docs::threshold
[`weighted`]: crate::docs::weighted
[`explicit`]: crate::docs::explicit
