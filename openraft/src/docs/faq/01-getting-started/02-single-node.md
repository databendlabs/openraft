### Are there any issues with running a single node service?

Not at all.

Running a cluster with just one node is a standard approach for testing or as an initial step in setting up a cluster.

A single node functions exactly the same as cluster mode.
It will consistently maintain the `Leader` status and never transition to `Candidate` or `Follower` states.
