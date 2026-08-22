# Jepsen Error-Handling Semantics

This document defines the agreed error-handling contract for OpenRaft Jepsen
runs. It specifies the behavior controlled by this suite. Current
implementation notes may describe gaps, but Client, Nemesis, and checker code
must preserve these boundaries.

## Classification Model

OpenRaft Jepsen recognizes only two runtime event classes:

- An **Outcome** is a result or observation recorded at a Workload or Nemesis
  boundary.
- A **Harness failure** is a failure in the test code, controller, configuration,
  dependencies, or execution environment.

Checker verdicts are derived from the recorded history and outcomes. They are
analysis results, not a third event class alongside Outcomes and Harness
failures. A checker can reject a run because it found a system under test (SUT)
property violation, insufficient coverage, or another failed postcondition.

Interruption is not a runtime event class. It is a lifecycle signal that must
propagate to the caller.

## Workload Outcomes

Workload outcomes include successful operations, modeled failures, and
ambiguous observations. Request timeouts and connection failures are valid
outcomes when they describe what a Client observed through the SUT API. For a
mutating request, such an observation may leave the operation's effect unknown.

`:unexpected-sut-response?` is an attribute on a Workload outcome, not a new
outcome class. Set it only when a valid request receives a SUT response outside
the modeled API contract, such as an unexpected HTTP response or invalid
response shape. Keep that outcome in the history so the
`unexpected-sut-responses` checker can report it and reject the run.

Do not set `:unexpected-sut-response?` on modeled failures or ambiguous
observations, including timeouts, connection failures, or quorum-related
rejections. Unknown Client, Nemesis, or other Harness exceptions must not
receive the marker. They are Harness failures.

## Nemesis Outcomes

A Nemesis operation has only three modeled outcomes:

- `installed` means the requested fault is known to have been installed, or the
  requested restoration is known to have completed.
- `skipped` means the action is known not to have been installed. Reasons
  include no safe target, no reachable leader, or a membership change already
  in progress.
- `indeterminate` means the action was attempted, but the available evidence
  cannot establish whether its side effect occurred.

There is no generic `failed` Nemesis outcome. A condition belongs in `skipped`
or `indeterminate` only when its meaning is explicitly modeled. An unclassified
exception or execution failure is a Harness failure.

The modeled `skipped` reasons assume a functioning control plane. Failure to
find or control a target because SSH failed is a Harness failure.

## Suite Harness Failures

Harness failures include:

- configuration, dependency, and environment failures;
- every SSH and other control-plane failure;
- unknown exceptions from Client, Nemesis, or checker code;
- command construction, permission, and environment-execution failures; and
- unexpected teardown or recovery failures.

This class covers boundaries where the OpenRaft suite can record a failure,
control subsequent work, and reach its aggregate checker. It does not include
Jepsen core-managed lifecycle failures.

SSH carries controller-to-node management traffic for setup, fault control,
recovery, and artifact collection. It is separate from Client traffic through
the SUT API and from node-to-node Raft traffic. An SSH authentication,
connection, session, or remote-execution failure is therefore always a Harness
failure, including when it occurs during a Nemesis operation. Uncertainty after
an SSH command was dispatched does not turn that failure into a Nemesis
`indeterminate` outcome.

## Controlled Stop After the First Suite Harness Failure

The first suite Harness failure starts a controlled stop:

1. Record the first failure as run-level state, including its source and
   original exception. Later failures must not replace it.
2. A gate around the ordinary runtime generator returns `nil` whenever Jepsen
   asks for another operation. This stops new ordinary Client and Nemesis
   operations from being scheduled.
3. Preserve completions from operations already in flight.
4. Run the final Nemesis recovery generator so installed faults can be removed
   and the cluster can recover.
5. Skip final Workload reads and writes. They cannot make a compromised run
   acceptable and would add new Workload history after the experiment stopped.
6. Run applicable analysis and retain its checker outputs.
7. Finish with a nonzero exit status.

The generator gate does not throw and does not perform cleanup. Returning `nil`
means normal generator exhaustion. It is neither an exception nor an
interruption. For a run that reaches normal generator completion, lifecycle
handling drains in-flight operations and then runs final recovery, suite
teardown, artifact collection, and applicable analysis.

## Teardown, Recovery, and Interruption

Membership and Recovery Nemeses have empty `teardown!` methods. Their formal
recovery belongs in final Nemesis operations.

Other Nemeses may perform cleanup during teardown. A failure in a non-empty
suite-controlled teardown is a Harness failure. Record it, then continue
independent cleanup,
artifact collection, and applicable analysis. Teardown must not exit early
because one cleanup failed, and the failure must not be reduced to a warning.

A modeled final-recovery outcome remains a Nemesis outcome, and a failed
recovery postcondition remains a checker verdict. An unexpected execution
failure while recovery is running is a Harness failure.

Interruption must propagate through Client, Nemesis, teardown, and recovery
code. Restore the thread's interruption state when required by the runtime and
rethrow the interruption. Do not convert it into an Outcome or Harness failure.

## `strict-unhandled-exceptions`

`strict-unhandled-exceptions` is a fallback diagnostic checker. An exception
that escapes a Client or Nemesis worker shows that the Harness missed a
classification boundary. The exception is a Harness failure.

The run-level first-failure state remains authoritative for runtime control.
This checker runs after teardown, when it cannot retroactively stop work or
recover the original `Throwable`. If it finds an escaped worker exception that
the run-level path missed, it emits structured post-hoc Harness evidence and
rejects the final verdict. It does not synthesize or mutate runtime failure
state from history.

Interruption is excluded from this rule and continues to propagate as a
lifecycle signal.

## Checker Exceptions

A non-interruption, non-fatal exception from checker implementation code is a
post-hoc Harness failure. The checker records serializable exception evidence
on its own result and returns a `false` verdict. This rule applies equally to a
checker run alone, a checker used as a composed child, and the composition
itself. Other independent checkers may still run and retain their results.

Checker exceptions do not mutate run-level failure state: analysis happens
after execution, so they cannot retroactively control scheduling or recover the
original runtime context.

## Run Acceptance

Any suite Harness failure makes the whole run unacceptable and requires a nonzero
exit. It does not erase recorded history, logs, nested checker results, or an
existing counterexample. A Harness failure does not by itself prove a SUT
property violation, and it does not invalidate a property violation that a
checker already established from retained evidence.

## Jepsen Core Lifecycle

Jepsen core owns OS and DB setup and teardown, Client and Nemesis lifecycle
setup, Client open and close, test-store creation, history and result
persistence, Harness log persistence, and remote artifact download. This suite
declares OpenRaft artifacts but does not wrap or replace that lifecycle.

An exception from a Jepsen core-managed lifecycle boundary propagates according
to Jepsen's native lifecycle semantics. It may stop the run before aggregate
analysis, so the suite does not record it as a suite Harness failure or promise
a checker verdict or `results.edn`. Artifact persistence failures follow the
same rule. Their handling belongs to Jepsen core.
