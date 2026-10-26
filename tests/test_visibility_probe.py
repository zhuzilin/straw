"""Check the measurement oracle against native framing and known stale views."""

import hashlib
import runpy
from pathlib import Path

import pytest

from straw import SharedFilesystemStore
from straw.protocol import RecordRef, SegmentRef

PROBE = runpy.run_path(
    str(Path(__file__).resolve().parents[1] / "verification/storage/probes/extent_visibility_probe.py")
)


def test_probe_layout_is_a_native_readable_extent(tmp_path):
    data, checksum = PROBE["framed_record"](7, 65536)
    path = tmp_path / "probe.pack"
    path.write_bytes(b"prefix" + data)
    segment = SegmentRef("visibility-probe", "probe.pack", "7", len(data), checksum, version=2, offset=6)
    with SharedFilesystemStore(tmp_path, "visibility-probe") as store:
        store.inspect_segment(segment, verify_payload=True)
        assert store.read_record(RecordRef(segment, 0)).payload == hashlib.shake_256(b"7").digest(65536)
    assert PROBE["validate_view"](data, len(data), checksum, 6 + len(data), 6 + len(data)) == []


@pytest.mark.parametrize("region", ["length", "header_zero", "header_partial", "footer", "index", "payload"])
def test_probe_distinguishes_stale_regions(region):
    data, checksum = PROBE["framed_record"](1, 1024)
    stale = bytearray(data)
    size = len(data)
    physical = size
    if region == "length":
        physical = size - 1
    elif region == "header_zero":
        stale[:12] = bytes(12)
    elif region == "header_partial":
        stale[3] = 0
    elif region == "footer":
        stale[-8:] = bytes(8)
    elif region == "index":
        index_size = int.from_bytes(data[-80:-72], "little")
        stale[size - 80 - index_size] = 0
    else:
        envelope_size = int.from_bytes(data[12:16], "little")
        stale[24 + envelope_size] ^= 255
    failures = PROBE["validate_view"](bytes(stale), size, checksum, physical, size)
    if region == "length":
        assert failures == ["short_stat"]
    elif region.startswith("header"):
        assert ("header_zero" if region == "header_zero" else "header_partial_zero") in failures
        assert "valid_header_with_later_failure" not in failures
    else:
        assert "valid_header_with_later_failure" in failures
        assert (region if region in {"footer", "index"} else "body_checksum") in failures


def test_probe_quantile_is_nearest_rank_and_empty_is_not_zero():
    assert PROBE["distribution"]([]) is None
    assert PROBE["distribution"](list(range(1, 1001)))["p99"] == 990
