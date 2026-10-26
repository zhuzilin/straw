----------------------- MODULE ExtentArithmetic -----------------------
EXTENDS Naturals, Integers
CONSTANTS WordMax, Kind, Mode
ASSUME /\ WordMax >= 31 /\ Kind \in {"frame", "usage"}
       /\ Mode \in {"fixed", "legacy"}
VARIABLES base, overhead, payload, done, result
vars == <<base, overhead, payload, done, result>>
\* A small unsigned word exhaustively exposes wraparound. Rust regressions use
\* the corresponding u64 boundaries; TLC integers are not machine u64 values.
Init == /\ base \in (IF Kind = "frame" THEN {12} ELSE {0, 1, WordMax})
        /\ overhead \in (IF Kind = "frame" THEN 12..15 ELSE {0, 1, WordMax})
        /\ payload \in 0..WordMax
        /\ done = FALSE /\ result = -1
Wrap(n) == n % (WordMax + 1)
Evaluate == /\ ~done /\ done' = TRUE
            /\ result' =
               (IF Mode = "fixed"
                THEN IF base + overhead + payload <= WordMax
                     THEN base + overhead + payload ELSE -1
                ELSE IF Kind = "frame"
                     THEN IF base + Wrap(overhead + payload) <= WordMax
                          THEN base + Wrap(overhead + payload) ELSE -1
                     ELSE Wrap(base + overhead + payload))
            /\ UNCHANGED <<base, overhead, payload>>
Spec == Init /\ [][Evaluate]_vars
TypeOK == /\ base \in 0..WordMax /\ overhead \in 0..WordMax
          /\ payload \in 0..WordMax /\ done \in BOOLEAN /\ result \in (-1)..WordMax
ExactOrRejected == done => result =
                   (IF base + overhead + payload <= WordMax
                    THEN base + overhead + payload ELSE -1)
=============================================================================
