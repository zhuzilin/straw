------------------------- MODULE EnvironmentLocalAccess -------------------------
EXTENDS Naturals
\* An acquired token is exclusive. Busy wins when an owner already holds the
\* lock; poison is reported when the mutex could otherwise be acquired.
EnvLocalAccess(held, poisoned) ==
    IF held THEN {"busy"} ELSE IF poisoned THEN {"poisoned"} ELSE {"acquired"}
=============================================================================
