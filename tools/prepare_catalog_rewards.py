#!/usr/bin/env python3
"""Bind shared verified reward facts to each map's actual captured NCS.

This is experimental preparation, not native gameplay acceptance. Missing map
flight paths can use explicitly reported compatibility routes. Original maps,
profiles, source captures and shared resource files are never overwritten.
"""
import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile

from prepare_game_state import CLIENT_SHA256, compute
from prepare_map_hud import map_entities
from prepare_flight_nodes import prepare as flight_nodes

CATALOG = Path(__file__).resolve().parents[1] / 'fixtures/protocol/killstreaks-3.3.3.3.json'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path, limit=16*1024*1024):
    path = Path(path)
    if not path.is_file() or path.stat().st_size > limit:
        raise ValueError('input missing or exceeds budget: ' + str(path))
    return path.read_bytes()


def resolve(base, value):
    return (base / value).resolve()


def compatibility_graph(center, provenance):
    specs = [('heli_start', [-3000, 0, 0], 1), ('ingress_end', [-1000, 0, 0], None),
             ('heli_loop_start', [-1000, -1000, 0], 3), ('patrol_2', [1000, -1000, 0], 4),
             ('patrol_3', [1000, 1000, 0], 5), ('patrol_4', [-1000, 1000, 0], 2),
             ('heli_leave', [2000, 0, 0], 7), ('exit', [5000, 0, 500], None)]
    return dict(sources_sha256=[provenance], nodes=[dict(name=name,
        origin=[max(-29000, min(29000, center[i]+delta[i])) for i in range(3)], target=target,
        script_airspeed=40.0, script_accel=22.0, script_delay=0.0) for name, delta, target in specs],
        entries=dict(heli_start=[0], heli_loop_start=[2], heli_leave=[6]))


def bind(state, ncs_raw, shared_raw, nuclear_raw, entities_raw=None,
         compatibility=False, unchecked=False, registry=None):
    registry = registry or json.loads(CATALOG.read_bytes())
    ncs, shared, nuclear = (json.loads(raw) for raw in [ncs_raw, shared_raw, nuclear_raw])
    if (ncs.get('schema_version') != 1 or ncs.get('read_only') is not True
            or ncs.get('client_sha256') != CLIENT_SHA256 or ncs.get('initialized') != 1
            or compute(ncs)['ncs_crc'] != state['ncs_crc']):
        raise ValueError('map NCS identity or CRC mismatch')
    for value in [state, shared, nuclear]:
        if value.get('schema_version') != 1 or value.get('client_sha256') != CLIENT_SHA256:
            raise ValueError('shared resource or map identity mismatch')
    if (shared.get('map'), shared.get('ncs_crc')) != (nuclear.get('map'), nuclear.get('ncs_crc')):
        raise ValueError('shared reward and nuclear sources must refer to the same captured map')
    if any(value for key, value in nuclear.items() if key.startswith('acceptance_')):
        raise ValueError('shared nuclear resources must not contain diagnostic progress or flags')

    def bank(category):
        return [base64.b64decode(s, validate=True).decode('ascii') for row in ncs['linked']
                if row['category'] == category for s in row['strings_base64']]

    def slot(category, name):
        values = bank(category)
        return values.index(name)+1 if values.count(name) == 1 else None

    def mapped(category, old):
        return {key: slot(category, key) for key in old if slot(category, key) is not None}

    reward_names = {r['native_id']: r['weapon_token_c13'] for r in registry['rewards']}
    old_weapons = {value['weapon']: name for name, value in shared['weapons'].items()}
    resource = copy.deepcopy(shared)
    resource.update(map=state['map'], ncs_crc=state['ncs_crc'], sources_sha256=[digest(shared_raw), digest(ncs_raw)],
        radios=[dict(r, weapon=slot(13, reward_names[r['native_id']])) for r in shared['radios'] if slot(13, reward_names[r['native_id']])],
        models=mapped(0, shared['models']), effects=mapped(3, shared['effects']), tags=mapped(9, shared.get('tags', {})),
        vehicles=mapped(2, shared.get('vehicles', {})), selectors={name: i+1 for i, name in enumerate(bank(12))}, weapons={})
    for key in ['model_bounds', 'camera_mounts']:
        resource[key] = {name: value for name, value in shared.get(key, {}).items() if name in resource['models']}
    for name, original in shared['weapons'].items():
        number = slot(13, name)
        if number is None:
            continue
        value = copy.deepcopy(original)
        value['weapon'] = number
        for key in ['clip_key', 'ammo_key']:
            old = value.get(key)
            if old in old_weapons and slot(13, old_weapons[old]) is not None:
                value[key] = slot(13, old_weapons[old])
        resource['weapons'][name] = value
    if resource.get('remote_exit'):
        resource['remote_exit']['material_bank_count'] = len(bank(8))

    entities = map_entities(json.loads(entities_raw), state['map']) if entities_raw else []
    points = [point for team in state['match_config']['spawns'] for point in team]
    if not points:
        raise ValueError('map requires prepared starts for compatible airspace')
    center = [sum(point[i] for point in points)/len(points) for i in range(3)]
    center[2] = min(25000, max(point[2] for point in points)+1800)
    anchors = [e for e in entities if e.get(b'targetname') == b'airstrikeheight']
    if len(anchors) == 1:
        center = [float(x) for x in anchors[0][b'origin'].split()]
    provenance = digest(entities_raw) if entities_raw else digest(json.dumps(state, sort_keys=True).encode())
    resource['air_route'] = dict(center=center, sources_sha256=[provenance])
    remote = []
    for spawn in entities:
        if spawn.get(b'targetname') != b'remoteMissileSpawn':
            continue
        targets = [e for e in entities if e.get(b'targetname') == spawn.get(b'target') and spawn.get(b'target')]
        if len(targets) != 1:
            raise ValueError('remote missile spawn requires one captured target')
        remote.append(dict(origin=[float(x) for x in spawn[b'origin'].split()],
                           target=[float(x) for x in targets[0][b'origin'].split()]))
    resource['remote_spawns'] = dict(sources_sha256=[provenance], points=remote)
    graph = flight_nodes(json.loads(entities_raw), state['map'], entities_raw) if entities_raw else None
    authored = graph is not None and all(key in graph['entries'] for key in ['heli_start', 'heli_loop_start', 'heli_leave'])
    if compatibility or not authored:
        center[2] = min(25000, max(center[2], max(point[2] for point in points)+2500))
        graph = compatibility_graph(center, provenance)
        authored = False
    resource['air_paths'] = graph
    if state['map'] == shared['map'] and not compatibility:
        for key in ['air_route', 'air_paths', 'remote_spawns', 'remote_vision']:
            if shared.get(key) is not None:
                resource[key] = copy.deepcopy(shared[key])
        authored = bool(shared.get('air_paths'))
        remote = resource['remote_spawns']['points']
    resource['experimental_air_routes'] = unchecked
    if not {1, 3, 5, 8, 16}.issubset({r['native_id'] for r in resource['radios']}):
        raise ValueError('map lacks required registered reward activators')
    if not {'vehicle_mi24p_hind_mp', 'projectile_hellfire_missile'}.issubset(resource['models']):
        raise ValueError('map lacks experimental reward models')

    result = copy.deepcopy(nuclear)
    result.update(map=state['map'], ncs_crc=state['ncs_crc'], sources_sha256=[digest(nuclear_raw), digest(ncs_raw)])
    bindings = [('weapon', 13, 'killstreak_nuke_mp'), ('damage_weapon', 13, 'nuke_mp'),
                ('flash_fx', 3, 'explosions/player_death_nuke_flash'), ('blast_fx', 3, 'explosions/player_death_nuke'),
                ('aftermath_fx', 3, 'dust/nuke_aftermath_mp'), ('tick_sound', 5, 'ui_mp_nukebomb_timer'),
                ('music_sound', 5, 'nuke_music'), ('earned_text', 4, 'MP_EARNED_NUKE'), ('called_text', 4, 'MP_TACTICAL_NUKE_CALLED')]
    for key, category, name in bindings:
        value = slot(category, name)
        if value is None:
            raise ValueError('missing or ambiguous nuclear resource: ' + name)
        result[key] = value + (511 if category == 5 else 0)
    report = dict(map=state['map'], mode=state['mode'], ncs_crc=state['ncs_crc'],
                  activators=len(resource['radios']), authored_flight_graph=authored,
                  authored_air_anchor=len(anchors) == 1, authored_missile_spawns=len(remote),
                  unchecked_air_routes=unchecked, experimental=True, native_acceptance_claimed=False)
    return resource, result, report


def prepare(catalog_path, capture_index, shared_path, nuclear_path, output,
            compatibility_maps=(), unchecked_maps=()):
    catalog_path, capture_index, output = map(lambda p: Path(p).resolve(), [catalog_path, capture_index, output])
    if output.exists():
        raise ValueError('output exists; choose a new preparation directory')
    catalog = json.loads(read(catalog_path, 65536))
    captures = json.loads(read(capture_index))
    if isinstance(captures, dict):
        if captures.get('schema_version') != 1:
            raise ValueError('capture index version mismatch')
        captures = captures['entries']
    lookup = {(entry['map'], entry['mode']): entry for entry in captures}
    if len(lookup) != len(captures) or catalog.get('schema_version') != 1 or not 1 <= len(catalog['entries']) <= 64:
        raise ValueError('catalog/index identity or entry count mismatch')
    shared_raw, nuclear_raw = read(shared_path, 256*1024), read(nuclear_path, 16384)
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.reward-preparation-', dir=output.parent))
    try:
        (staging/'maps').mkdir()
        (staging/'rewards').mkdir()
        entries, reports, seen = [], [], set()
        for entry in catalog['entries']:
            name, mode = entry['map'], entry['mode']
            if not re.fullmatch(r'[a-z0-9_]{1,64}', name) or mode not in ('war', 'cha') or (name, mode) in seen:
                raise ValueError('invalid or duplicate map/mode')
            seen.add((name, mode))
            source = resolve(catalog_path.parent, entry['file'])
            state = json.loads(read(source, 256*1024))
            if (state.get('map'), state.get('mode')) != (name, mode):
                raise ValueError('map preparation identity mismatch')
            capture = lookup[(name, mode)]
            entity_path = capture.get('entities', capture.get('map-entities'))
            entities_raw = read(resolve(capture_index.parent, entity_path), 8*1024*1024) if entity_path else None
            radar, nuclear, report = bind(state, read(resolve(capture_index.parent, capture['ncs'])), shared_raw, nuclear_raw,
                entities_raw, name in compatibility_maps, name in unchecked_maps)
            for key, value in list(state.items()):
                if key.endswith('_file') and value:
                    state[key] = str(resolve(source.parent, value))
            for key, value, suffix in [('radar_file', radar, 'radar'), ('nuke_file', nuclear, 'nuke')]:
                relative = 'rewards/%s-%s-%s.json' % (name, mode, suffix)
                (staging/relative).write_text(json.dumps(value, indent=2)+'\n')
                state[key] = '../'+relative
            relative = 'maps/%s-%s.json' % (name, mode)
            (staging/relative).write_text(json.dumps(state, indent=2)+'\n')
            entries.append(dict(map=name, mode=mode, file=relative))
            reports.append(report)
        (staging/'catalog.json').write_text(json.dumps(dict(catalog, entries=entries), indent=2)+'\n')
        (staging/'rewards-report.json').write_text(json.dumps(reports, indent=2)+'\n')
        staging.rename(output)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return output/'catalog.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['catalog', 'capture-index', 'shared-resources', 'shared-nuke', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--compatibility-airspace', default='', help='comma-separated maps using generated routes')
    parser.add_argument('--unchecked-air-routes', default='', help='comma-separated experimental maps allowed to attempt blocked preflight routes')
    args = parser.parse_args()
    result = prepare(args.catalog, args.capture_index, args.shared_resources, args.shared_nuke, args.output,
        set(filter(None, args.compatibility_airspace.split(','))), set(filter(None, args.unchecked_air_routes.split(','))))
    print(json.dumps(dict(catalog=str(result), experimental=True, native_acceptance_claimed=False)))


if __name__ == '__main__':
    main()
