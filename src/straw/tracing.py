"""Optional payload-free I/O traces, one append file per process.

Set STRAW_TRACE_DIR before starting the application. Traces are observational,
not part of queue durability. No tensor values or application metadata are logged.
"""

import atexit
import json
import os
import socket
import threading
import time
from pathlib import Path

_lock = threading.Lock()
_stream = None
_pid = None
_directory = None


def emit(operation, *, root, size, seconds, records=None, checked_bytes=None):
    directory = os.environ.get("STRAW_TRACE_DIR")
    if not directory:
        return
    global _stream, _pid, _directory
    with _lock:
        if _stream is None or _pid != os.getpid() or _directory != directory:
            if _stream is not None:
                _stream.close()
            _directory = directory
            Path(directory).mkdir(parents=True, exist_ok=True)
            _pid = os.getpid()
            _stream = (Path(directory) / f"{socket.gethostname()}-{_pid}.jsonl").open("a", buffering=1)
            atexit.register(_stream.close)
        event = {
            "op": operation,
            "time": time.time(),
            "pid": _pid,
            "host": socket.gethostname(),
            "root": str(root),
            "bytes": size,
            "seconds": seconds,
        }
        if checked_bytes is not None:
            event["checked_bytes"] = checked_bytes
        if records is not None:
            event["records"] = [
                {
                    "bytes": r.payload.nbytes if isinstance(r.payload, memoryview) else len(r.payload),
                    "codec": r.codec,
                    "tokens": r.tokens,
                    **{k: r.metadata[k] for k in ("shape", "dtype", "kind") if k in r.metadata},
                }
                for r in records
            ]
        _stream.write(json.dumps(event, separators=(",", ":")) + "\n")
