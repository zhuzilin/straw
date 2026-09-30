"""Small JSON control transport for CPU examples and independent-host admission.

The training adapter can host Coordinator in its existing Ray actor instead.
No sample payloads cross this transport. It has a single request loop.
"""

import dataclasses
import hmac
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

from . import errors
from .protocol import Lease, RecordSetRef, TaskSpec, encode

METHODS = {
    "submit_tasks",
    "acquire",
    "heartbeat",
    "release_tasks",
    "complete_task",
    "fail_task",
    "cancel_task",
    "seal_input",
    "lookup_submission",
    "read_commits",
    "task_status",
    "metrics",
    "drain",
    "open_consumer",
    "close_consumer",
    "save_consumer_state",
    "load_consumer_state",
    "plan_batch",
    "batch_ready",
    "get_batch",
    "register_checkpoint",
    "producer_state",
    "yield_task",
    "save_task_progress",
    "save_task_progress_many",
}


def serve(coordinator, address, *, token, ready=None):
    if not token:
        raise ValueError("A job-scoped RPC token is required")

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.connection.settimeout(10)
            if not hmac.compare_digest(self.headers.get("Authorization", ""), f"Bearer {token}"):
                self.send_error(403)
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if length <= 0:
                    raise ValueError("RPC request must have a positive Content-Length")
                request = json.loads(self.rfile.read(length))
                method, args = request["method"], request["args"]
                if method not in METHODS:
                    raise ValueError("Unknown control operation")
                if "lease" in args:
                    args["lease"] = Lease(**args["lease"])
                if "leases" in args:
                    args["leases"] = [Lease(**value) for value in args["leases"]]
                if "tasks" in args:
                    args["tasks"] = [TaskSpec.from_dict(value) for value in args["tasks"]]
                for key in (
                    "result_ref",
                    "state_ref",
                    "ready_ref",
                    "plan_ref",
                    "checkpoint_ref",
                    "input_ref",
                ):
                    if key in args:
                        args[key] = RecordSetRef.from_dict(args[key])
                result = getattr(coordinator, method)(**args)
                if dataclasses.is_dataclass(result):
                    result = dataclasses.asdict(result)
                response = encode({"result": result})
                status = 200
            except (errors.QueueError, ValueError, KeyError, TypeError) as error:
                response = encode({"error": type(error).__name__, "message": str(error)})
                status = 409
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            try:
                self.wfile.write(response)
            except (BrokenPipeError, ConnectionResetError):
                pass  # Durable completion is discoverable via the original submission ID.

        def log_message(self, *args):
            pass

    class ControlServer(HTTPServer):
        request_queue_size = 128

    with ControlServer(address, Handler) as server:
        if ready:
            ready(server.server_address)
        server.serve_forever(poll_interval=0.2)


class QueueClient:
    def __init__(self, address, *, token, timeout=30, metadata_bytes=256 * 1024):
        self.address, self.token, self.timeout, self.metadata_bytes = (
            address,
            token,
            timeout,
            metadata_bytes,
        )
        self.opener = build_opener(ProxyHandler({}))

    def call(self, method, **args):
        for key, value in args.items():
            if dataclasses.is_dataclass(value):
                args[key] = dataclasses.asdict(value)
            elif isinstance(value, (list, tuple)):
                args[key] = [dataclasses.asdict(item) if dataclasses.is_dataclass(item) else item for item in value]
        data = encode({"method": method, "args": args})
        request = Request(
            self.address,
            data=data,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
        )
        try:
            response = self.opener.open(request, timeout=self.timeout)
        except HTTPError as error:
            response = error
        with response:
            content = response.read()
            value = json.loads(content)
        if "error" in value:
            cls = getattr(errors, value["error"], errors.QueueError)
            if not isinstance(cls, type) or not issubclass(cls, Exception):
                cls = errors.QueueError
            raise cls(value["message"])
        return value["result"]
