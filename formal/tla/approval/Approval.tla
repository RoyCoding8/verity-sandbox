------------------------------ MODULE Approval ------------------------------
EXTENDS Naturals, Sequences

CONSTANTS Modes, UseExpectedOid

\* Two attempts, two candidate identities, two existing branches, one external action.
\* Candidate identity stands in for a validated review digest. Handoff records and
\* Git objects are immutable after reading. Publication replaces one complete record.
Attempts == {"A", "B"}
Branches == {"target", "other"}
Evidence == {"accepted", "inconclusive", "rejected"}
NoRecord == [candidate |-> "none", base |-> "none", evidence |-> "none"]
Fresh == [phase |-> "load", record |-> NoRecord, digest |-> "none",
          expected |-> "none", merged |-> "none", route |-> "none",
          result |-> "none", oid |-> "none", written |-> "none", before |-> "none",
          headBranchAtEffect |-> "none"]
Event(a, kind, data) == <<a, kind, data>>

VARIABLES mode, refs, branch, away, index, disk, handoffs, published,
          attempts, environmentAction, event

vars == <<mode, refs, branch, away, index, disk, handoffs, published,
          attempts, environmentAction, event>>

State == <<mode, refs, branch, away, index, disk, handoffs, published,
           attempts, environmentAction>>

Slot(a) == IF mode = "repeat" THEN "A" ELSE a

InitialPlace(place) ==
    /\ branch = IF place = "target" THEN "target" ELSE "other"
    /\ away = (place = "elsewhere")
    /\ refs = [r \in Branches |-> "base"]
    /\ index = <<"base", "clean">>
    /\ disk = <<"base", "clean">>

Init ==
    /\ mode \in Modes
    /\ \E place \in {"target", "other", "elsewhere"}: InitialPlace(place)
    /\ handoffs = [a \in Attempts |-> NoRecord]
    /\ published = {}
    /\ attempts = [a \in Attempts |-> Fresh]
    /\ environmentAction = "none"
    /\ event = Event("env", "initial", <<>>)

LocalState == <<refs, branch, index, disk>>

HeadOid == refs[branch]
Snapshot == <<refs["target"], refs["other"], branch,
              index[1], index[2], disk[1], disk[2]>>

Publish(a, evidence) ==
    LET record == [candidate |-> a, base |-> "base", evidence |-> evidence] IN
    /\ a \notin published
    /\ published' = published \cup {a}
    /\ handoffs' = [handoffs EXCEPT ![Slot(a)] = record]
    /\ event' = Event(a, "publish", <<record.candidate, record.base, evidence, a>>)
    /\ UNCHANGED <<mode, away, LocalState, attempts, environmentAction>>

ApprovalStep(a, kind, data, nextAttempt) ==
    /\ attempts' = [attempts EXCEPT ![a] = nextAttempt]
    /\ event' = Event(a, kind, data)
    /\ UNCHANGED <<mode, away, handoffs, published, environmentAction>>

ObserveStep(a, kind, data, nextAttempt) ==
    /\ ApprovalStep(a, kind, data, nextAttempt)
    /\ UNCHANGED LocalState

ReadHandoff(a) ==
    LET attempt == attempts[a]
        record == handoffs[Slot(a)]
    IN
    /\ attempt.phase = "load"
    /\ ObserveStep(a, "load", <<record.candidate, record.base, record.evidence,
                                 record.candidate>>,
          [attempt EXCEPT !.record = record, !.phase = "gate"])

Gate(a, digest) ==
    LET attempt == attempts[a]
        record == attempt.record
        result == CASE record = NoRecord -> "missing"
                       [] record.candidate # digest -> "digest"
                       [] record.evidence # "accepted" -> "gate"
                       [] OTHER -> "none"
    IN
    /\ attempt.phase = "gate"
    /\ ObserveStep(a, "gate", <<digest, result>>,
          [attempt EXCEPT !.digest = digest, !.result = result,
                          !.phase = IF result = "none" THEN "readTarget" ELSE "finish"])

ReadTarget(a) ==
    LET attempt == attempts[a] IN
    /\ attempt.phase = "readTarget"
    /\ ObserveStep(a, "target", <<refs["target"]>>,
          [attempt EXCEPT !.expected = refs["target"], !.phase = "ancestry"])

MergeOids(c) == {"merge" \o c, "merge" \o c \o "2"}
Includes(oid, c) == oid \in MergeOids(c)

CheckAncestry(a) ==
    LET attempt == attempts[a]
        included == Includes(attempt.expected, attempt.record.candidate)
        result == IF included THEN "already"
                  ELSE IF attempt.expected # attempt.record.base THEN "moved"
                  ELSE "none"
    IN
    /\ attempt.phase = "ancestry"
    /\ ObserveStep(a, "ancestry", <<attempt.expected, attempt.record.candidate,
                                    IF included THEN "included" ELSE "absent">>,
          [attempt EXCEPT !.result = result,
                          !.oid = IF included THEN attempt.expected ELSE "none",
                          !.phase = IF result = "none" THEN "tree" ELSE "finish"])

MergeTree(a, gitResult) ==
    LET attempt == attempts[a]
        nextPhase == IF gitResult = "clean" THEN "commit" ELSE "finish"
        result == CASE gitResult = "clean" -> "none"
                       [] gitResult = "conflict" -> "conflict"
                       [] OTHER -> "gitError"
    IN
    /\ attempt.phase = "tree"
    /\ ObserveStep(a, "tree", <<gitResult>>,
          [attempt EXCEPT !.phase = nextPhase, !.result = result])

CreateCommit(a) ==
    LET attempt == attempts[a]
        candidate == attempt.record.candidate
    IN
    /\ attempt.phase = "commit"
    /\ \E oid \in IF a = candidate THEN {"merge" \o candidate} ELSE MergeOids(candidate):
        ObserveStep(a, "commit", <<oid>>,
          [attempt EXCEPT !.merged = oid, !.phase = "lookup"])

FindCheckout(a) ==
    LET attempt == attempts[a]
        owner == CASE branch = "target" -> "local"
                      [] away -> "elsewhere"
                      [] OTHER -> "unchecked"
    IN
    /\ attempt.phase = "lookup"
    /\ ObserveStep(a, "lookup", <<owner>>,
          [attempt EXCEPT !.route = IF owner = "unchecked" THEN "cas" ELSE "local",
                          !.phase = IF owner = "unchecked" THEN "cas" ELSE "symbolic"])

ReadSymbolicHead(a) ==
    LET attempt == attempts[a]
        same == branch = "target"
    IN
    /\ attempt.phase = "symbolic"
    /\ ObserveStep(a, "symbolic", <<branch>>,
          [attempt EXCEPT !.phase = IF same THEN "head" ELSE "finish",
                          !.result = IF same THEN "none" ELSE "gitError"])

ReadHeadOid(a) ==
    LET attempt == attempts[a]
        same == HeadOid = attempt.expected
    IN
    /\ attempt.phase = "head"
    /\ ObserveStep(a, "head", <<HeadOid>>,
          [attempt EXCEPT !.phase = IF same THEN "status" ELSE "finish",
                          !.result = IF same THEN "none" ELSE "gitError"])

TrackedClean == index = <<HeadOid, "clean">> /\ disk = index

CheckTracked(a) ==
    LET attempt == attempts[a]
        clean == TrackedClean
    IN
    /\ attempt.phase = "status"
    /\ ObserveStep(a, "status", <<IF clean THEN "clean" ELSE "dirty">>,
          [attempt EXCEPT !.phase = IF clean THEN "merge" ELSE "finish",
                          !.result = IF clean THEN "none" ELSE "gitError"])

Integration(a, kind, writtenRef, gitResult) ==
    LET attempt == attempts[a]
        wrote == writtenRef # "none"
        ok == gitResult # "refused"
    IN
    /\ ApprovalStep(a, kind, <<gitResult>>,
          [attempt EXCEPT !.phase = "finish",
                          !.result = IF ok THEN "new" ELSE "gitError",
                          !.oid = IF ok THEN attempt.merged ELSE "none",
                          !.written = writtenRef,
                          !.headBranchAtEffect = branch,
                          !.before = IF wrote THEN refs[writtenRef] ELSE "none"])
    /\ refs' = IF wrote THEN [refs EXCEPT ![writtenRef] = attempt.merged] ELSE refs
    /\ UNCHANGED branch

CompareAndSwap(a) ==
    LET attempt == attempts[a]
        matches == refs["target"] = attempt.expected
    IN
    /\ attempt.phase = "cas"
    /\ \/ /\ matches \/ ~UseExpectedOid
          /\ Integration(a, "cas", "target", "applied")
       \/ Integration(a, "cas", "none", "refused")
    /\ UNCHANGED <<index, disk>>

\* Index and disk each record the committed tree identity and one tracked edit.
\* Fast-forward abstracts a completed Git command. It includes clean application,
\* retained unrelated edits, an already-current HEAD, and refusal without a write.
\* Interrupted Git commands and untracked or ignored paths are outside this bound.
FastForward(a) ==
    LET attempt == attempts[a]
        ready == HeadOid = attempt.expected
                 /\ index[1] = HeadOid /\ disk[1] = HeadOid
                 /\ index[2] # "affected" /\ disk[2] # "affected"
    IN
    /\ attempt.phase = "merge"
    \* Git acts on its current HEAD. A preceding observation cannot pin it.
    /\ \/ /\ ready
          /\ Integration(a, "merge", branch, "applied")
          /\ index' = <<attempt.merged, index[2]>>
          /\ disk' = <<attempt.merged, disk[2]>>
       \/ /\ HeadOid = attempt.merged
          /\ Integration(a, "merge", "none", "unchanged")
          /\ UNCHANGED <<index, disk>>
       \/ /\ Integration(a, "merge", "none", "refused")
          /\ UNCHANGED <<index, disk>>

Finish(a, logged) ==
    LET attempt == attempts[a] IN
    /\ attempt.phase = "finish"
    /\ ~logged \/ attempt.result \in {"new", "conflict"}
    /\ ObserveStep(a, "return", <<attempt.result, attempt.oid,
                                   IF logged THEN "logged" ELSE "unlogged">>,
          [attempt EXCEPT !.phase = "observe"])

ObserveEffects(a) ==
    /\ attempts[a].phase = "observe"
    /\ ObserveStep(a, "observe", Snapshot, [attempts[a] EXCEPT !.phase = "done"])

External(action, data) ==
    /\ environmentAction = "none"
    /\ environmentAction' = action
    /\ event' = Event("env", action, data)
    /\ UNCHANGED <<mode, away, handoffs, published, attempts>>

AdvanceRef(r) ==
    /\ refs[r] # "external"
    /\ External("advance", <<r, "external">>)
    /\ refs' = [refs EXCEPT ![r] = "external"]
    /\ UNCHANGED <<branch, index, disk>>

SwitchCheckout ==
    LET destination == IF branch = "target" THEN "other" ELSE "target" IN
    /\ ~away
    /\ TrackedClean
    /\ External("switch", <<destination>>)
    /\ branch' = destination
    /\ index' = <<refs[destination], "clean">>
    /\ disk' = index'
    /\ UNCHANGED refs

Edit(kind, staged) ==
    /\ External("edit", <<kind, IF staged THEN "staged" ELSE "unstaged">>)
    /\ index' = IF staged THEN <<index[1], kind>> ELSE index
    /\ disk' = <<disk[1], kind>>
    /\ UNCHANGED <<refs, branch>>

Approval(a) ==
    \/ ReadHandoff(a)
    \/ \E digest \in Attempts: Gate(a, digest)
    \/ ReadTarget(a)
    \/ CheckAncestry(a)
    \/ \E outcome \in {"clean", "conflict", "gitError"}: MergeTree(a, outcome)
    \/ CreateCommit(a)
    \/ FindCheckout(a)
    \/ ReadSymbolicHead(a)
    \/ ReadHeadOid(a)
    \/ CheckTracked(a)
    \/ CompareAndSwap(a)
    \/ FastForward(a)
    \/ \E logged \in BOOLEAN: Finish(a, logged)
    \/ ObserveEffects(a)

Next ==
    \/ \E a \in Attempts: Approval(a)
    \/ \E a \in Attempts, evidence \in Evidence: Publish(a, evidence)
    \/ \E r \in Branches: AdvanceRef(r)
    \/ SwitchCheckout
    \/ \E kind \in {"affected", "unrelated"}, staged \in BOOLEAN: Edit(kind, staged)

Spec == Init /\ [][Next]_vars

ReviewedEvidence ==
    \A a \in Attempts:
        LET attempt == attempts[a] IN
        attempt.result \in {"new", "already", "conflict"} =>
            /\ attempt.record.evidence = "accepted"
            /\ attempt.digest = attempt.record.candidate
            /\ IF attempt.result = "already"
               THEN Includes(attempt.oid, attempt.record.candidate)
               ELSE attempt.expected = attempt.record.base
            /\ (attempt.result = "new" => Includes(attempt.oid, attempt.record.candidate))

PreservesRefUpdates ==
    \A a \in Attempts:
        LET attempt == attempts[a] IN
        attempt.written # "none" => attempt.before = attempt.expected

RepeatHasNoWrite ==
    \A a \in Attempts:
        LET attempt == attempts[a] IN
        attempt.result = "already" =>
            /\ attempt.oid = attempt.expected
            /\ attempt.written = "none"

\* Checkout ownership can change after lookup. The intended ref and checkout
\* effect guarantee requires stable ownership through integration.
StableCheckoutEffects ==
    environmentAction # "switch" =>
        \A a \in Attempts:
            LET attempt == attempts[a] IN
            attempt.written # "none" =>
                /\ attempt.written = "target"
                /\ (attempt.route = "cas" => attempt.headBranchAtEffect # "target")

=============================================================================
