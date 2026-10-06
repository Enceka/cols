"""Flatten the target's runtime StructuredDataDef; never use old-build offsets."""
import struct

WIDTH = {0: 4, 1: 1, 2: 1, 4: 2, 8: 4, 9: 2}


def flatten(definition, reject_overlaps=False):
    """Return named scalar leaves after checking leaf coverage and byte bounds."""
    d = definition
    fields = {}
    seen = set()
    occupied = set()

    def leaves(kind, index, trail=()):
        key = (kind, index)
        if key in trail or len(trail) > 32:
            raise ValueError("recursive/over-deep schema")
        if kind == 5:
            return d["structs"][index]["leafCount"]
        if kind == 6:
            a = d["indexedArrays"][index]
            return a["count"] * leaves(a["type"], a["index"], trail + (key,))
        if kind == 7:
            a = d["enumedArrays"][index]
            count = max(e["value"] for e in d["enums"][a["enum"]]["entries"]) + 1
            return count * leaves(a["type"], a["index"], trail + (key,))
        if kind not in WIDTH and kind != 3:
            raise ValueError("unknown schema type")
        return 1

    def walk(kind, index, path, leaf, projected=False, depth=0):
        if depth > 32:
            raise ValueError("schema recursion limit")
        if kind == 5:
            for p in d["structs"][index]["properties"]:
                if not p["name"]:
                    raise ValueError("unnamed runtime property")
                walk(p["type"], p["index"], path + ("." if path else "") + p["name"],
                     leaf + p["leaf"], projected or bool(p["extra4"] & 32), depth + 1)
        elif kind in (6, 7):
            a = d["indexedArrays" if kind == 6 else "enumedArrays"][index]
            if kind == 6:
                keys = [(i, str(i)) for i in range(a["count"])]
            else:
                keys = [(e["value"], e["name"]) for e in d["enums"][a["enum"]]["entries"]]
            stride = leaves(a["type"], a["index"])
            for i, label in keys:
                walk(a["type"], a["index"], path + "[" + label + "]", leaf + i * stride, projected, depth + 1)
        else:
            if leaf in seen or not 0 <= leaf < d["leafCount"] or path in fields:
                raise ValueError("duplicate/out-of-range leaf")
            seen.add(leaf)
            raw = d["offsets"][leaf]
            offset = 4 + (raw // 8 if kind == 2 else raw)
            width = index if kind == 3 else WIDTH[kind]
            if width <= 0 or offset < 12 or offset + width > 4 + d["size"] or offset + width > 32768:
                raise ValueError("field outside profile: " + path)
            first_bit = offset * 8 + (raw % 8 if kind == 2 else 0)
            bits = set(range(first_bit, first_bit + (1 if kind == 2 else width * 8)))
            if reject_overlaps and occupied.intersection(bits):
                raise ValueError("overlapping fields: " + path)
            occupied.update(bits)
            fields[path] = {"type": kind, "index": index, "leaf": leaf, "offset": offset,
                            "width": width, "bit": raw % 8 if kind == 2 else None, "projected": projected}

    if len(d["offsets"]) != d["leafCount"] or leaves(d["rootType"], d["rootIndex"]) != d["leafCount"]:
        raise ValueError("inconsistent root size")
    walk(d["rootType"], d["rootIndex"], "", 0)
    if seen != set(range(d["leafCount"])):
        raise ValueError("schema expansion does not cover every leaf")
    return fields


def value(snapshot, field):
    at = field["offset"]
    kind = field["type"]
    if kind == 2:
        return bool(snapshot[at] & (1 << field["bit"]))
    if kind == 3:
        return snapshot[at:at + field["width"]].split(b"\0", 1)[0]
    return struct.unpack_from({0: "<i", 1: "<B", 4: "<H", 8: "<f", 9: "<h"}[kind], snapshot, at)[0]
