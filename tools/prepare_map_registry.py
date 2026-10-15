#!/usr/bin/env python3
"""Derive native TDM map identities without treating metadata as prepared resources."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re

from clientlib import manifest


def prepare(capture, mode='war'):
    if mode not in ('war', 'cha'):
        raise ValueError('unsupported native team mode')
    expected = next(f['sha256'] for f in manifest()['files']
                    if f['path'] == 'CODOL/codoMP_client_shipRetail.exe')
    if capture.get('schema_version') != 1 or capture.get('read_only') is not True or capture.get('client_sha256') != expected:
        raise ValueError('expected current-client read-only map table')
    table = capture['tables']['mp/coMapList.csv']
    if (not table.get('found') or table['columns'] != 37 or
            not 1 <= table['rows'] <= 128 or len(table['cells_base64']) != table['rows'] * 37):
        raise ValueError('native map table layout differs')
    cells = [base64.b64decode(c, validate=True).decode('ascii') for c in table['cells_base64']]
    result = []
    ids, names = set(), set()
    for i in range(0, len(cells), 37):
        row = cells[i:i + 37]
        if row[8] != 'pvp' or mode not in row[9].split(';'):
            continue
        name, native_id = row[4], int(row[3])
        minimum, maximum = int(row[10]), int(row[11])
        no_custom = int(row[35] or '0')
        if (not re.fullmatch(r'(?:mp|za)_[a-z0-9_]{1,60}', name) or
                not 1 <= native_id <= 512 or any(flag not in ('0', '1') for flag in row[:3]) or
                native_id in ids or name in names or not 1 <= minimum <= maximum <= 32 or no_custom not in (0, 1)):
            raise ValueError('invalid or ambiguous native TDM map identity')
        ids.add(native_id)
        names.add(name)
        result.append({'name': name, 'native_id': native_id, 'native_flags': list(map(int, row[:3])),
                       'label_key': row[5], 'modes': row[9].split(';'), 'character_rules': row[36],
                       'minimum_players': minimum, 'maximum_players': maximum, 'no_custom_room': bool(no_custom)})
    if not result:
        raise ValueError('native table has no TDM maps')
    return {'schema_version': 1, 'client_sha256': expected, 'table': 'mp/coMapList.csv',
            'table_cells_sha256': hashlib.sha256(b'\0'.join(s.encode('ascii') for s in cells)).hexdigest(),
            'resource_preparation_claimed': False, 'maps': result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--mode', choices=('war','cha'), default='war')
    args = parser.parse_args()
    raw = args.assets.read_bytes()
    result = prepare(json.loads(raw), args.mode)
    result['capture_sha256'] = hashlib.sha256(raw).hexdigest()
    with args.output.open('x') as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write('\n')
    print(json.dumps({'output': str(args.output), 'tdm_maps': len(result['maps'])}))


if __name__ == '__main__':
    main()
