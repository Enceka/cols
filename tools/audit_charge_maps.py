#!/usr/bin/env python3
"""Audit native assault spawn pools without borrowing another mode's points.

The capture index maps native map names to entity_capture/entity_sha256 records.
Re-read each source rather than trusting saved class counts. Catalog membership
only describes preparation; native gameplay acceptance is recorded separately.
"""
import argparse
import hashlib
import json
from pathlib import Path

from clientlib import REPO
from prepare_game_state import CLIENT_SHA256
from prepare_map_hud import map_entities
from prepare_tdm_map import charge_spawns


def audit(index, catalog):
    registry = json.loads((REPO / 'fixtures/protocol/charge-map-registry-3.3.3.3.json').read_text())
    if registry['client_sha256'] != CLIENT_SHA256 or registry['resource_preparation_claimed'] is not False:
        raise ValueError('unexpected native registry identity')
    prepared = {(entry['map'], entry['mode']) for entry in catalog['entries']}
    rows = []
    for native in registry['maps']:
        name = native['name']
        row = dict(map=name, native_id=native['native_id'], native_flags=native['native_flags'])
        rows.append(row)
        if native['native_flags'][2] != 1:
            row['status'] = 'excluded_version'
            continue
        if name not in index:
            row['status'] = 'unobserved'
            continue
        source = index[name]
        path = Path(source['entity_capture'])
        if not path.is_absolute():
            path = REPO / path
        raw = path.read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != source['entity_sha256']:
            raise ValueError('entity source hash mismatch: ' + name)
        entities = map_entities(json.loads(raw), name)
        counts = {}
        for entity in entities:
            classname = entity.get(b'classname', b'').decode('ascii')
            if classname.startswith('mp_cha_spawn_'):
                counts[classname] = counts.get(classname, 0) + 1
        source_label = str(path.relative_to(REPO)) if path.is_relative_to(REPO) else str(path)
        row.update(entity_capture=source_label, entity_sha256=digest, spawn_classes=counts)
        enough = all(counts.get('mp_cha_spawn_' + team, 0) >= 1 and
                     (counts.get('mp_cha_spawn_' + team + '_start', 0) or
                      counts.get('mp_cha_spawn_' + team, 0)) >= 1
                     for team in ('axis', 'allies'))
        if not enough:
            if (name, 'cha') in prepared:
                raise ValueError('catalog includes map without complete native assault pools: ' + name)
            row['status'] = 'excluded_no_native_pools'
            continue
        starts, bases = charge_spawns(json.loads(raw), name)
        row.update(opening_counts=[len(team) for team in starts], base_counts=[len(team) for team in bases],
                   status='prepared' if (name, 'cha') in prepared else 'eligible_unprepared')
    return dict(schema_version=1, client_sha256=CLIENT_SHA256, build_version_column=2,
                native_gameplay_claimed=False, maps=rows,
                complete=all(row['status'] not in ('unobserved', 'eligible_unprepared') for row in rows))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture-index', type=Path, required=True)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = audit(json.loads(args.capture_index.read_text()), json.loads(args.catalog.read_text()))
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(dict(complete=report['complete'], maps={row['map']: row['status'] for row in report['maps']})))


if __name__ == '__main__':
    main()
