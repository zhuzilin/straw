---------------------- MODULE EnvironmentPreparation ----------------------
\* ENV-PREPARATION: a fresh private name is never reused or written by peers.
\* Register/open failures may already have created the durable catalog entry
\* or an empty file. No payload has been written before offset capture.
EnvPreparationFailure(before) == {before, TRUE}
\* Successful unlink (including truthful NotFound) removes this private name.
EnvRemovedPrivate == FALSE
=============================================================================
