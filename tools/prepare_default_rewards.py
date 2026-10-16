"""Fill empty native backpack reward selections while preserving existing loadouts."""
import json
from pathlib import Path
import struct
import zlib
from prepare_inventory import public_snapshot

CATALOG = Path(__file__).resolve().parents[1] / 'fixtures/protocol/killstreaks-3.3.3.3.json'
DEFAULT_REWARDS = ('uav', 'predator_missile', 'helicopter')


def default_items(catalog=None):
    catalog = catalog or json.loads(CATALOG.read_bytes())
    result = []
    for name in DEFAULT_REWARDS:
        matches = [row['item_id'] for row in catalog['marketplace'] if row['name'] == name and row['native_id'] is not None]
        if len(matches) != 1 or not 1 <= matches[0] <= 65535:
            raise ValueError('default reward is absent or ambiguous: ' + name)
        result.append(matches[0])
    return result


def seed_empty_rewards(arguments, fields, catalog=None):
    catalog = catalog or json.loads(CATALOG.read_bytes())
    items = default_items(catalog)
    snapshot, tail = public_snapshot(arguments)
    projected = [f for f in fields.values() if f['projected']]
    expected = b''.join(b'\x07' + snapshot[f['offset']:f['offset']+4] for f in projected) + b'\0'
    if len(projected) != 125 or expected != tail:
        raise ValueError('public projection does not match snapshot')
    offsets = catalog['class_offsets']
    if len(offsets) != 60:
        raise ValueError('expected sixty native backpack records')
    edited = bytearray(snapshot)
    changed = []
    for index, row in enumerate(offsets):
        for slot, at in enumerate(row[:3]):
            field = fields.get('weaponClasses[%d].killstreaks[%d]' % (index, slot))
            if not field or (field['type'], field['width'], field['offset'], field['projected']) != (0, 4, at, False):
                raise ValueError('killstreak field does not match the target schema')
        values = [struct.unpack_from('<i', snapshot, at)[0] for at in row[:3]]
        if not all(value in (0, 1) for value in values):
            continue
        for at, item in zip(row[:3], items):
            struct.pack_into('<i', edited, at, item + 1)
        changed.append(index)
    if not changed:
        return arguments, items, changed
    crc = zlib.crc32(edited[4:])
    struct.pack_into('<I', edited, 0, crc)
    blob = zlib.compress(edited, 1)
    u32 = lambda value: b'\x08' + struct.pack('<I', value)
    result = u32(1) + u32(1) + u32(crc) + b'\x13' + u32(len(blob)) + blob + tail
    public_snapshot(result)
    return result, items, changed


def repair_service_data(assets, root, apply=False):
    """Plan or back up/apply empty selections in stopped local service data."""
    import base64
    import hashlib
    import os
    import secrets
    import shutil
    from schema_fields import flatten

    client = 'e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b'
    if assets.get('read_only') is not True or assets.get('client_sha256') != client:
        raise ValueError('verified target schema capture required')
    definition = assets['schemas']['mp/playerdata.def']['definitions'][0]
    if (definition['version'], definition['formatChecksum']) != (233, 3811397846):
        raise ValueError('public schema mismatch')
    fields = flatten(definition)
    root = Path(root).resolve()
    changes, skipped = [], []
    template = root/'new-player-template.json'
    if template.is_file():
        original = template.read_bytes()
        value = json.loads(original)
        if value.get('schema_version') != 1 or value.get('client_sha256') != client:
            raise ValueError('player template identity mismatch')
        public, items, classes = seed_empty_rewards(base64.b64decode(value['public_base64'], validate=True), fields)
        if not set(items).issubset(value['all_items']):
            raise ValueError('default rewards absent from complete item catalogue')
        starter = list(dict.fromkeys(value['starter_items'] + items))
        if classes or starter != value['starter_items']:
            value['public_base64'] = base64.b64encode(public).decode()
            value['starter_items'] = starter
            changes.append((template, original, json.dumps(value, separators=(',', ':')).encode(), classes))
    for path in sorted((root/'profiles').glob('*/*/public.bin')):
        original = path.read_bytes()
        result, items, classes = seed_empty_rewards(original, fields)
        if not classes:
            continue
        inventory = path.parent/'inventory.json'
        if not inventory.is_file() or not set(items).issubset(json.loads(inventory.read_text())['items']):
            skipped.append(dict(file=str(path.relative_to(root)), reason='default_rewards_not_owned'))
            continue
        changes.append((path, original, result, classes))
    backup = None
    if apply and changes:
        backup = root/('backup-default-rewards-'+secrets.token_hex(8))
        for path, original, _, _ in changes:
            target = backup/path.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(original)
        for path, original, replacement, _ in changes:
            if hashlib.sha256(path.read_bytes()).digest() != hashlib.sha256(original).digest():
                raise ValueError('profile changed during offline repair; backup preserved')
            temporary = path.with_name('.'+path.name+'.defaults-'+secrets.token_hex(8)+'.tmp')
            try:
                with temporary.open('xb') as stream:
                    stream.write(replacement)
                    stream.flush()
                    os.fsync(stream.fileno())
                shutil.copymode(path, temporary)
                temporary.replace(path)
            finally:
                temporary.unlink(missing_ok=True)
    return dict(applied=apply, backup=str(backup) if backup else None,
                changes=[dict(file=str(path.relative_to(root)), classes=classes) for path, _, _, classes in changes], skipped=skipped)


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--service-data-dir', type=Path, required=True)
    parser.add_argument('--apply', action='store_true', help='apply only after stopping the client and backend; originals are backed up')
    args = parser.parse_args()
    if args.assets.stat().st_size > 16*1024*1024:
        parser.error('schema capture exceeds budget')
    print(json.dumps(repair_service_data(json.loads(args.assets.read_text()), args.service_data_dir, args.apply), indent=2))


if __name__ == '__main__':
    main()
