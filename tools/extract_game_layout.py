#!/usr/bin/env python3
"""Extract the target's static playerState field order (no code execution)."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

import pefile
from clientlib import DEFAULT_WORKSPACE, checked_sample, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=DEFAULT_WORKSPACE)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = checked_sample(args.workspace, "CODOL/codoMP_client_shipRetail.exe")
    pe = pefile.PE(str(source), fast_load=True)
    table = pe.get_data(0xc55f70 - pe.OPTIONAL_HEADER.ImageBase, 235 * 12)
    fields = [list(values) for values in struct.iter_unpack("<iii", table)]
    if len(fields) != 235 or any(not 0 <= offset < 0x3524 or not -101 <= bits <= 32
                                  or not 0 <= hint <= 3 for offset, bits, hint in fields):
        raise ValueError("target playerState table layout mismatch")
    data = dict(schema_version=1, client_sha256=sha256(source), table_va=0xc55f70,
                table_sha256=hashlib.sha256(table).hexdigest(), player_state_bytes=0x3524,
                columns=["offset", "codec_bits", "hint"], fields=fields)
    # One tuple per line keeps the measured wire order readable.
    header = json.dumps({k:v for k,v in data.items() if k != "fields"}, indent=2)
    rendered = header[:-2] + ',\n  "fields": [\n' + ',\n'.join('    '+json.dumps(f) for f in fields) + '\n  ]\n}\n'
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(rendered)
    print(json.dumps({"output":str(args.output), "fields":len(fields), "table_sha256":data["table_sha256"]}))


if __name__ == "__main__":
    main()
