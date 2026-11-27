--------------------------- MODULE ApprovalReplay ---------------------------
EXTENDS Approval, ApprovalTrace

VARIABLES trace, cursor

replayVars == <<vars, trace, cursor>>

ReplayInit ==
    /\ Init
    /\ trace \in Traces
    /\ mode = trace.mode
    /\ InitialPlace(trace.place)
    /\ cursor = 0

ReplayNext ==
    \/ /\ cursor = Len(trace.events)
       /\ UNCHANGED replayVars
    \/ /\ cursor < Len(trace.events)
       /\ cursor' = cursor + 1
       /\ UNCHANGED trace
       /\ Next
       /\ event' = trace.events[cursor']

ReplaySpec == ReplayInit /\ [][ReplayNext]_replayVars

TraceHasAttempts ==
    /\ Len(trace.events) > 0
    /\ trace.attempts # {}
    /\ trace.attempts \subseteq Attempts

Complete ==
    cursor = Len(trace.events) =>
        \A a \in Attempts:
            attempts[a].phase = IF a \in trace.attempts THEN "done" ELSE "load"

=============================================================================
