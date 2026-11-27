---- MODULE Restore ----
EXTENDS Naturals, Sequences

\* One code-and-chat Revert, two changed file positions, a disjoint sibling,
\* and an initially absent redo record. Redo can be retried across sessions.
\* snapshots.rs preflights before per-file application, then pi_ipc.rs calls
\* Pi. Compensation and Redo compare affected paths to the restored tree.
\* The model checks separate file and metadata effects. A process may stop
\* between actions. Reopen takes session identity as input; it does not model
\* Pi's session files, interrupted syscalls, or power-loss durability.
\* Concurrent Verity processes and edits after preflight are outside the bound.

CONSTANT SkipStaleCheck
VARIABLE state

Files(version) == <<version, version>>
Before == Files("Before")
Checkpoint == Files("Checkpoint")

Init ==
    state = [phase |-> "RevertPreflight", event |-> "Step", files |-> Before,
             sibling |-> "Before", session |-> "Parent", redo |-> FALSE,
             wrote |-> 0, start |-> Before, caller |-> "Child", crashed |-> FALSE]

Preflight ==
    /\ state.phase = "RevertPreflight"
    /\ \/ state' = [state EXCEPT !.phase = "RevertApply", !.event = "Step"]
       \/ state' = [state EXCEPT !.phase = "Idle", !.event = "PreflightRefused"]

\* Each successful removal or checkout advances one file. I/O failure can
\* follow an already completed file. Git checkout may unlink the old file
\* before a required smudge filter fails.
Apply(phase, version, finishingPhase, event) ==
    /\ state.phase = phase
    /\ state.wrote < Len(state.files)
    /\ LET position == state.wrote + 1
           done == position = Len(state.files)
       IN state' = [state EXCEPT !.files[position] = version,
                    !.wrote = position, !.phase = IF done THEN finishingPhase ELSE phase,
                    !.event = IF done THEN event ELSE "Step"]

ApplyFailed(phase, event) ==
    /\ state.phase = phase
    /\ \E missing \in BOOLEAN :
        state' = [state EXCEPT !.phase = "Idle", !.event = event,
                  !.files = IF missing
                            THEN [state.files EXCEPT ![state.wrote + 1] = "Missing"]
                            ELSE state.files]

RequestFork ==
    /\ state.phase = "RequestFork"
    /\ state' = [state EXCEPT !.phase = "WaitFork", !.event = "PiFork"]

ForkFirstTurnRejected ==
    /\ state.phase = "WaitFork"
    /\ state' = [state EXCEPT !.phase = "WaitNewSession", !.event = "PiNewSession"]

PiAccepted ==
    /\ state.phase \in {"WaitFork", "WaitNewSession"}
    /\ state' = [state EXCEPT !.phase = "SaveRedo", !.event = "PiAccepted",
                 !.session = "Child"]

PiRejected ==
    /\ state.phase \in {"WaitFork", "WaitNewSession"}
    /\ state' = [state EXCEPT !.phase = "CompensatePreflight", !.event = "PiRejected",
                 !.start = state.files, !.wrote = 0]

\* The independent writer can act while Pi is outstanding or between requests.
\* Each edit is to one relevant path or one disjoint sibling before preflight.
IndependentEdit ==
    /\ state.phase \in {"WaitFork", "WaitNewSession", "Idle"}
    /\ \/ /\ state.files[1] # "Edited"
          /\ state' = [state EXCEPT !.files[1] = "Edited", !.event = "EditedPath"]
       \/ /\ state.sibling # "Edited"
          /\ state' = [state EXCEPT !.sibling = "Edited", !.event = "EditedSibling"]

CompensatePreflight ==
    /\ state.phase = "CompensatePreflight"
    /\ IF state.files # Checkpoint
       THEN state' = [state EXCEPT !.phase = "Idle", !.event = "CompensationStale"]
       ELSE \/ state' = [state EXCEPT !.phase = "CompensateApply", !.event = "Step"]
            \/ state' = [state EXCEPT !.phase = "Idle", !.event = "CompensationPreflightRefused"]

SaveRedo ==
    /\ state.phase = "SaveRedo"
    /\ \E saved \in BOOLEAN :
        state' = [state EXCEPT !.phase = "Idle", !.redo = saved,
                  !.event = IF saved THEN "RedoSaved" ELSE "RedoSaveFailed"]

\* A caller can select a different session between requests. The record is
\* owned by Child and binds this Revert's two tree identities and file set.
StartRedo ==
    /\ state.phase = "Idle"
    /\ \E caller \in {"Parent", "Child"} :
        state' = [state EXCEPT !.phase = "RedoPreflight", !.event = "Step",
                  !.start = state.files, !.wrote = 0, !.caller = caller]

RedoPreflight ==
    /\ state.phase = "RedoPreflight"
    /\ IF ~state.redo
       THEN state' = [state EXCEPT !.phase = "Idle", !.event = "RedoUnavailable"]
       ELSE IF state.caller # "Child"
       THEN state' = [state EXCEPT !.phase = "Idle", !.event = "RedoWrongOwner"]
       ELSE IF state.files # Checkpoint /\ ~SkipStaleCheck
       THEN state' = [state EXCEPT !.phase = "Idle", !.event = "RedoStale"]
       ELSE \/ state' = [state EXCEPT !.phase = "RedoApply", !.event = "Step"]
            \/ state' = [state EXCEPT !.phase = "Idle", !.event = "RedoPreflightRefused"]

ClearRedo ==
    /\ state.phase = "ClearRedo"
    /\ \E cleared \in BOOLEAN :
        state' = [state EXCEPT !.phase = "Idle", !.redo = ~cleared,
                  !.event = IF cleared THEN "RedoCleared" ELSE "RedoClearFailed"]

StopProcess ==
    /\ state.phase \notin {"Stopped", "Idle"}
    /\ ~state.crashed
    /\ state' = [state EXCEPT !.phase = "Stopped", !.event = "Crash", !.crashed = TRUE]

Reopen ==
    /\ state.phase = "Stopped"
    /\ \E session \in {"Parent", "Child"} :
        state' = [state EXCEPT !.phase = "Idle", !.event = "Reopened", !.session = session]

Next ==
    \/ Preflight
    \/ Apply("RevertApply", "Checkpoint", "RequestFork", "Step")
    \/ ApplyFailed("RevertApply", "ApplyFailed")
    \/ RequestFork \/ ForkFirstTurnRejected \/ PiAccepted \/ PiRejected
    \/ IndependentEdit \/ CompensatePreflight
    \/ Apply("CompensateApply", "Before", "Idle", "Compensated")
    \/ ApplyFailed("CompensateApply", "CompensationApplyFailed")
    \/ SaveRedo \/ StartRedo \/ RedoPreflight
    \/ Apply("RedoApply", "Before", "ClearRedo", "Step")
    \/ ApplyFailed("RedoApply", "RedoApplyFailed")
    \/ ClearRedo \/ StopProcess \/ Reopen

Spec == Init /\ [][Next]_state

Observe(s) == <<s.event, s.files, s.sibling, s.session, s.redo>>

ApplyRequiresFreshness ==
    /\ (state.phase = "CompensateApply" => state.start = Checkpoint /\ ~state.redo)
    /\ (state.phase \in {"RedoApply", "ClearRedo"} =>
        state.start = Checkpoint /\ state.caller = "Child" /\ state.redo)

RefusalsRetainFilesAndMetadata ==
    /\ (state.event = "PreflightRefused" => state.files = Before /\ ~state.redo)
    /\ (state.event \in {"CompensationStale", "CompensationPreflightRefused"} =>
        state.files = state.start /\ ~state.redo)
    /\ (state.event \in {"RedoWrongOwner", "RedoStale", "RedoPreflightRefused"} =>
        state.files = state.start /\ state.redo)
    /\ (state.event = "RedoUnavailable" => state.files = state.start /\ ~state.redo)

CompensationDoesNotPublishRedo ==
    state.event = "Compensated" => state.files = Before /\ ~state.redo /\ state.session = "Parent"

ReportedOutcomes ==
    /\ (state.event = "PiFork" => state.files = Checkpoint)
    /\ (state.event \in {"RedoSaved", "RedoSaveFailed"} =>
        state.session = "Child" /\ state.wrote = Len(state.files) /\
        state.redo = (state.event = "RedoSaved"))
    /\ (state.event \in {"RedoCleared", "RedoClearFailed"} =>
        state.files = Before /\ state.redo = (state.event = "RedoClearFailed"))

PreserveSibling ==
    [][(state'.event # "EditedSibling" => state'.sibling = state.sibling)]_state

====
