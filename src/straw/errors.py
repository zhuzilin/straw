"""Stable failure categories for the filesystem queue (no framework dependencies)."""


class QueueError(RuntimeError):
    pass


class LeaseExpired(QueueError):
    pass


class StaleAttempt(QueueError):
    pass


class IdempotencyConflict(QueueError):
    pass


class IndeterminateCommit(QueueError):
    pass


class StorageUnavailable(QueueError):
    pass


class QuotaExceeded(StorageUnavailable):
    pass


class ResourceLimitExceeded(QueueError):
    pass


class CorruptData(QueueError):
    pass


class UnsupportedSchema(QueueError):
    pass


class InvalidReference(QueueError):
    pass


class UnsafeRecovery(QueueError):
    pass
