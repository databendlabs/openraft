### Fast Commit(obsolete)

**This algorithm is no longer used**, because it would be more flexible treating
local-IO the same as replication to a remote node. As a result, applying a log does not
need to wait for it to be flushed on local disk.

#### Fast Commit Algorithm

The algorithm assumes that an entry will be committed as soon as it is appended
if the cluster has only one voter.

However, if there is a membership log in the middle of the input entries, the
condition to commit will change, and entries before and after the membership
entry must be dealt with differently.

When a membership entry is seen, update progress for all former entries.
Then upgrade the quorum set for the Progress.

E.g., if the input entries are `2..6`, entry 4 changes membership from `a` to `abc`.
Then it will output a LeaderCommit command to commit entries `2,3`.

```text
1 2 3 4 5 6
a x x a y y
      b
      c
```

If the input entries are `2..6`, entry 4 changes membership from `abc` to `a`.
Then it will output a LeaderCommit command to commit entries `2,3,4,5,6`.

```text
1 2 3 4 5 6
a x x a y y
b
c
```
