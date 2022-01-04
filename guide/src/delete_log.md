When appending logs or installing snapshot(another form of appending logs),
Conflicting logs **should** be removed.

To maintain log consistency, it does not need to delete.
Because:
If newer logs(with higher`(term, index)`) are sent from some leader,
the incompatible logs must be uncommitted.
Further election will never choose these logs,
if the logs from the new leader are committed.

But membership changing requires it to remove the incompatible logs when appending.
This is what raft did wrongly.

If membership logs are kept in a separate log column, there wont be such a problem.
But membership a mixed into the business log.
which makes it a non-WAL storage.

The following is an exmaple of why logs must be removed:
A membership change log will only be appended if the previous membership log is committed,
which is the algo raft defined.
A non-committed membership log is only allowed to present if any quorum is in its intersection with a quorum in the committed config.

But when new logs are appended, there could be a membership log that is not compatible with a membership log in the incompatible logs.
which results in a split-brain.

E.g.:

R1 becomes leader in term 1
R1 appends two log and one membership log m3
R1 crash and recovered.
R3 becomes leader in term 2
R3 appends 2 membership log m'1,m'2 and committed. // Joint membership change algo
R3 send log m'1 and m'2 to R1.

Then m'2 and m3 could be incompatible:
m0 = {R1,R2,R3},
m'1 = {R3,X,Y} x {R1,R2,R3} // joint
m'2 = {R3,X,Y} // final
m3 = {R1,U,V} x {R1,R2,R3} 

m3 is compatible with the committed membership log when it is created.
But not compatible with others'

```
R1 L1 e1,e2,m3            m'1,m'2,m3
R2 F1          F2 m'1, 
R3             L2 m'1,m'2
X                 m'1,m'2      
Y                 
--------------------------> time
```

Thus this is the reason logs must be removed.
Or move all membership log to another column is also a clean option.