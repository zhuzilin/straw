----------------------- MODULE LeaseSelection -----------------------
EXTENDS Naturals, Sequences, FiniteSets, EnvironmentInputs, EnvironmentProgress
CONSTANTS Tasks, MaxFilter, Mode
ASSUME /\ Tasks # {} /\ MaxFilter >= 2 /\ Mode \in {"fixed", "legacy"}
VARIABLES input, limit, cursor, output
vars == <<input, limit, cursor, output>>
Init == /\ input \in EnvFilterInputs(Tasks, MaxFilter)
        /\ limit \in 1..MaxFilter /\ cursor = 1 /\ output = <<>>
Seen == {input[j] : j \in 1..(cursor-1)}
Step == /\ cursor <= Len(input) /\ Len(output) < limit
        /\ output' = IF Mode = "legacy" \/ input[cursor] \notin Seen
                       THEN Append(output, input[cursor]) ELSE output
        /\ cursor' = cursor + 1 /\ UNCHANGED <<input, limit>>
Spec == Init /\ [][Step]_vars /\ EnvWeakFairness(vars, Step)
TypeOK == /\ input \in Seq(Tasks) /\ output \in Seq(Tasks)
          /\ cursor \in 1..(Len(input)+1) /\ limit \in 1..MaxFilter
UniqueAssignments == Cardinality({output[i] : i \in 1..Len(output)}) = Len(output)
LimitRespected == Len(output) <= limit
CompleteSelection == cursor > Len(input) =>
    {output[i] : i \in 1..Len(output)} = {input[i] : i \in 1..Len(input)}
Terminates == <> (cursor > Len(input) \/ Len(output) = limit)
=============================================================================
