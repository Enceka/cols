#!/usr/bin/env python3
"""Verify capture continuity and content hashes. Does not decode game packets."""
import argparse
import json
import sys
from pathlib import Path

from clientlib import sha256


def check_session(events_path):
    binary = events_path.with_suffix(".bin")
    offset, chunks, started, ended = 0, 0, False, False
    summary = None
    with events_path.open(encoding="utf-8") as source:
        for line in source:
            event = json.loads(line)
            kind = event.get("event")
            if ended:
                raise ValueError("event after session_end")
            if kind == "session_start":
                if started or event.get("schema_version") != 1:
                    raise ValueError("duplicate start or unknown schema")
                started = True
                if event.get("transport") != "tcp" or event.get("direction") != "client_to_server":
                    raise ValueError("unsupported capture transport/direction")
            elif kind == "stream_chunk":
                length = event.get("length")
                if not started or event.get("offset") != offset or type(length) is not int or length <= 0:
                    raise ValueError("invalid or non-contiguous stream chunk")
                offset += length
                chunks += 1
            elif kind == "session_end":
                if not started or event.get("bytes") != offset or event.get("chunks") != chunks:
                    raise ValueError("invalid session totals")
                if binary.stat().st_size != offset or sha256(binary) != event.get("sha256"):
                    raise ValueError("capture content size/hash mismatch")
                ended = True
                summary = {"file": binary.name, "bytes": offset, "chunks": chunks,
                           "reason": event.get("reason"), "sha256": event["sha256"]}
            else:
                raise ValueError("unknown event: " + str(kind))
    if not ended:
        raise ValueError("unfinished capture: no session_end")
    return summary


def check_run(directory):
    metadata = json.loads((directory / "run.json").read_text(encoding="utf-8"))
    if metadata.get("schema_version") != 1 or metadata.get("mode") != "tcp_observe_no_response":
        raise ValueError("unsupported run schema/mode")
    events = sorted(directory.glob("*.jsonl"))
    raw = {p.stem for p in directory.glob("*.bin")}
    if raw != {p.stem for p in events}:
        raise ValueError("orphan binary/event file")
    return {"verified": True, "scope": "capture_integrity_only",
            "sessions": [check_session(p) for p in events]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        result = check_run(args.directory)
    except (OSError, ValueError, KeyError, TypeError) as error:
        result = {"verified": False, "error": str(error)}
    print(json.dumps(result, indent=2))
    return 0 if result["verified"] else 1


if __name__ == "__main__":
    sys.exit(main())
