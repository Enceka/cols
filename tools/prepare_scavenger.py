#!/usr/bin/env python3
"""Derive native scavenger identity and ITEM layout from this fixed client."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct

import pefile
from clientlib import DEFAULT_WORKSPACE, checked_sample, sha256


def prepare(source, capture_path):
    raw = capture_path.read_bytes()
    capture = json.loads(raw)
    digest = sha256(source)
    if capture.get('client_sha256') != digest or capture.get('read_only') is not True:
        raise ValueError('expected current-client read-only registry capture')
    bags = capture['supply_definitions']
    if len(bags) != 1 or bags[0]['name'] != 'scavenger_bag_mp':
        raise ValueError('scavenger definition missing or ambiguous')
    bag = bags[0]
    definition = base64.b64decode(bag['definition_base64'], validate=True)
    if len(definition) != 0xe28 or struct.unpack_from('<I', definition, 0x488)[0] != 5:
        raise ValueError('unexpected scavenger inventory type')
    model = next(m for m in bag['world_models'] if m['slot'] == 0)
    header = base64.b64decode(model['header_base64'], validate=True)
    center = struct.unpack_from('<fff', header, 0x140)
    half = struct.unpack_from('<fff', header, 0x14c)
    pe = pefile.PE(str(source), fast_load=True)
    read = lambda address, length: pe.get_data(address - pe.OPTIONAL_HEADER.ImageBase, length)
    table = read(0xc543b8, 61 * 12)
    fields = [list(row) for row in struct.iter_unpack('<iii', table)]
    if fields[19] != [0x98, 11, 0] or fields[23] != [8, -98, 0] or fields[30] != [0x80, 11, 0]:
        raise ValueError('native ITEM descriptor differs')
    # x87 FLD/FCOMP uses qword constants, not float32.
    touch = [struct.unpack('<d', read(at, 8))[0] for at in (0xc85f68, 0xc85f70, 0xc85f60, 0xc85a68)]
    if touch != [-36.0, 36.0, -88.0, 18.0]:
        raise ValueError('native touch constants differ')
    if bag['touch_constants'] != touch:
        raise ValueError('runtime touch constants differ from the fixed executable')
    return dict(schema_version=1, client_sha256=digest, capture_sha256=hashlib.sha256(raw).hexdigest(),
                name=bag['name'], weapon=bag['index'], world_model=model['name'],
                definition_sha256=hashlib.sha256(definition).hexdigest(),
                bounds_min=[a-b for a, b in zip(center, half)], bounds_max=[a+b for a, b in zip(center, half)],
                pickup_sounds=bag['pickup_sounds'], entity_type=3, table_va=0xc543b8,
                table_sha256=hashlib.sha256(table).hexdigest(), fields=fields,
                touch_delta_min=[touch[0], touch[0], touch[2]], touch_delta_max=[touch[1], touch[1], touch[3]],
                pickup_event=16)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', type=Path, default=DEFAULT_WORKSPACE)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = prepare(checked_sample(args.workspace, 'CODOL/codoMP_client_shipRetail.exe'), args.capture)
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    print(json.dumps({'output': str(args.output), 'world_model': result['world_model'], 'weapon': result['weapon']}))


if __name__ == '__main__':
    main()
