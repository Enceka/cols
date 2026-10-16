"""Recover authored aircraft graphs from matching, read-only map entities."""
import hashlib
import math
from prepare_map_hud import map_entities


ROOTS = ('heli_start', 'heli_loop_start', 'heli_leave', 'heli_crash_start',
         'heli_attack_area', 'heli_dest')


def prepare(capture, map_name, source_raw):
    entities = map_entities(capture, map_name)
    named = {}
    for index, entity in enumerate(entities):
        if b'targetname' in entity:
            named.setdefault(entity[b'targetname'], []).append(index)
    roots = {name: named.get(name.encode(), []) for name in ROOTS}
    pending = [index for indices in roots.values() for index in indices]
    if not pending:
        return None
    selected = set()
    while pending:
        index = pending.pop()
        if index in selected:
            continue
        selected.add(index)
        if len(selected) > 256:
            raise ValueError('aircraft graph exceeds node budget')
        target = entities[index].get(b'target')
        if target:
            matches = named.get(target, [])
            if len(matches) != 1:
                raise ValueError('missing or ambiguous aircraft target: ' + target.decode('ascii'))
            pending.append(matches[0])
    indices = sorted(selected)
    remap = {old: new for new, old in enumerate(indices)}
    nodes = []
    for index in indices:
        entity = entities[index]
        name = entity[b'targetname'].decode('ascii')
        if not 1 <= len(name) <= 64:
            raise ValueError('invalid aircraft node name')
        origin = [float(value) for value in entity.get(b'origin', b'').split()]
        if len(origin) != 3 or not all(math.isfinite(value) and abs(value) <= 30000 for value in origin):
            raise ValueError('invalid aircraft node origin')
        node = dict(name=name, origin=origin, target=remap[named[entity[b'target']][0]] if entity.get(b'target') else None)
        if b'angles' in entity:
            angles = [float(value) for value in entity[b'angles'].split()]
            if len(angles) != 3 or not all(math.isfinite(value) and abs(value) <= 3600 for value in angles):
                raise ValueError('invalid aircraft node angles')
            node['angles'] = angles
        elif b'angle' in entity:
            angle = float(entity[b'angle'])
            if not math.isfinite(angle) or abs(angle) > 3600:
                raise ValueError('invalid aircraft node yaw')
            node['angles'] = [0.0, angle, 0.0]
        for field in ('script_airspeed', 'script_accel', 'script_delay'):
            value = entity.get(field.encode())
            if value is not None:
                value = float(value)
                if not math.isfinite(value) or not 0 <= value <= (600 if field == 'script_delay' else 50000):
                    raise ValueError('invalid aircraft node ' + field)
                node[field] = value
        nodes.append(node)
    return dict(sources_sha256=[hashlib.sha256(source_raw).hexdigest()], nodes=nodes,
                entries={name: [remap[index] for index in group] for name, group in roots.items() if group})
