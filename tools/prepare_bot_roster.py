#!/usr/bin/env python3
"""Bind current-version Bot classes to captured weapons, outfits and animations.

Writes a new preparation. Native asynchronous loading can resolve captured DB placeholders. The
optional mounted-only mode reports fallbacks; model names always come from
the actual class and character variant tables.
"""
import argparse
import base64
import copy
import hashlib
import json
import re
import struct
from pathlib import Path

from prepare_bot_locomotion import actor_animations, blocks, rawfile, select_action
from prepare_game_state import CLIENT_SHA256
from prepare_player_appearance import GROUPS, native_hash


def rows(table):
    if table.get("found") is not True:
        raise ValueError("required native table unavailable")
    cells = [base64.b64decode(x, validate=True).decode("ascii") for x in table["cells_base64"]]
    width = table["columns"]
    if not 1 <= width <= 256 or len(cells) != width * table["rows"]:
        raise ValueError("native table dimensions mismatch")
    return [cells[i:i + width] for i in range(0, len(cells), width)]


def variant(table, item, gender, faction):
    candidates = []
    for ordinal, row in enumerate(rows(table)):
        if row[0].lower() != item.lower() or row[1] != "model":
            continue
        selector = row[2].split()
        if len(selector) != 3:
            raise ValueError("unsupported character selector")
        g, f, tone = selector
        if g not in {gender, "s"} or f not in {faction, "ss"}:
            continue
        score = (g == gender) + 2 * (f == faction) + (tone == "sss")
        candidates.append((score, -ordinal, row[3].lower()))
    if not candidates:
        raise ValueError("native character variant unavailable: " + item)
    return max(candidates)[2]


def animation_config(template, capture, definition):
    script_bytes = rawfile(capture, "mp/playeranim.script")
    script = re.sub(r"//[^\n]*|/\*.*?\*/", "", script_bytes.decode("ascii"), flags=re.S)
    combat = [body for header, body in blocks(script) if header == "state combat"]
    if len(combat) != 1:
        raise ValueError("native combat rules ambiguous")
    types = rawfile(capture, "mp/playeranimtypes.txt").decode("ascii").split()
    at, wc = struct.unpack_from("<I", definition, 0x478)[0], struct.unpack_from("<I", definition, 0x480)[0]
    classes = {0: "rifle", 1: "sniper", 2: "mg", 3: "smg", 4: "spread", 5: "pistol"}
    if at >= len(types) or wc not in classes:
        raise ValueError("unsupported Bot weapon animation class")
    context = {"playeranimtype": types[at].lower(), "playeranimtypeprimary": types[at].lower(),
               "weaponclass": classes[wc], "weapon_position": "hip", "strafing": "not", "movetype": "idle"}
    indices = {}
    for a in actor_animations(capture):
        indices.setdefault(a["name"].lower(), set()).add(a["index"])

    def index(name):
        found = indices.get(name.lower(), set())
        if len(found) != 1 or not 0 < next(iter(found)) < 1024:
            raise ValueError("native Bot animation missing or ambiguous: " + name)
        return next(iter(found))

    config = copy.deepcopy(template)
    config.pop("fire_animation_msec", None)
    locomotion = {}
    for field, move, strafe in [
        ("walk_animation", "walk", "not"), ("walk_back_animation", "walkbk", "not"),
        ("walk_left_animation", "walk", "left"), ("walk_right_animation", "walk", "right"),
        ("run_forward", "run", "not"), ("run_back", "runbk", "not"),
        ("run_left", "run", "left"), ("run_right", "run", "right"), ("sprint", "sprint", "not"),
    ]:
        action = select_action(combat[0], move, dict(context, movetype=move, strafing=strafe))
        if action is None or action[0] != "both":
            raise ValueError("unsupported native Bot movement action")
        (config if field.startswith("walk_") else locomotion)[field] = index(action[1])
    fire = select_action(script, "fireweapon", context, with_duration=True)
    if fire is None or fire[0] != "torso" or select_action(script, "fireweapon", dict(context, movetype="walk")) is not None:
        raise ValueError("unsupported native Bot firing overlay")
    locomotion["fire_standing"] = index(fire[1])
    duration = fire[2]
    if duration is None:
        found = [a.get("duration_msec") for a in actor_animations(capture)
                 if a["name"].lower() == fire[1].lower()]
        duration = found[0] if len(found) == 1 else None
    if duration is not None:
        if not isinstance(duration, int) or not 1 <= duration <= 15000:
            raise ValueError("invalid captured Bot firing action duration")
        config["fire_animation_msec"] = duration
    elif definition[0xd74] != 0:
        raise ValueError("bolt-action body animation duration was not captured")
    reload = select_action(script, "reload", context)
    if (reload is None or reload[0] != "torso"
            or reload != select_action(script, "reload", dict(context, movetype="walk"))):
        raise ValueError("unsupported native Bot reload overlay")
    config["reload_animation"] = index(reload[1])
    locomotion["script_sha256"] = hashlib.sha256(script_bytes).hexdigest()
    config["locomotion"] = locomotion
    return config


def model_registry(state, capture):
    # A later character capture also contains dynamically registered outfits.
    # Only the inherited prefix plus explicit registrations in this preparation
    # can be reused in another session; do not inherit a stale player's slot.
    registered=state.get("match_config",{}).get("models",[])
    first=min((i for i,_ in registered),default=max(m["model"] for m in capture["models"])+1)
    models={m["name"].lower():m["model"] for m in capture["models"] if m["model"]<first}
    ids={i:n for n,i in models.items()}
    for index,name in registered:
        name=name.lower()
        if (index in ids and ids[index]!=name) or (name in models and models[name]!=index):
            raise ValueError("conflicting explicit Bot model registration")
        ids[index]=name;models[name]=index
    return models,list(registered)


def rebind_animations(state, capture, arsenal):
    """Bind each retained Bot's motions to its current weapon definition."""
    for value in (state, capture, arsenal):
        if value["client_sha256"] != CLIENT_SHA256 or value["ncs_crc"] != state["ncs_crc"]:
            raise ValueError("Bot capture client/map identity mismatch")
    if capture.get("read_only") is not True or arsenal.get("read_only") is not True:
        raise ValueError("Bot animation preparation requires read-only captures")
    weapons = {w["name"]: w for w in arsenal["weapons"]}
    if len(weapons) != len(arsenal["weapons"]):
        raise ValueError("duplicate captured Bot weapon")
    roster = copy.deepcopy(state["match_config"]["roster"])
    for bot in roster:
        weapon = weapons.get(bot["weapon_name"])
        if weapon is None:
            raise ValueError("Bot weapon not captured: " + bot["weapon_name"])
        definition = base64.b64decode(weapon["definition_base64"], validate=True)
        if len(definition) != 0xe28:
            raise ValueError("native weapon layout mismatch")
        bot["animation"] = animation_config(bot["animation"], capture, definition)
    state["match_config"]["roster"] = roster
    return {"rebound_bot_animations": len(roster)}


class NativeBotCharacters:
    """Resolve the existing outfit through the retained native item banks."""
    def __init__(self, capture):
        if capture.get("client_sha256") != CLIENT_SHA256 or capture.get("read_only") is not True:
            raise ValueError("native Bot character capture identity mismatch")
        raw = lambda key: base64.b64decode(capture[key], validate=True)
        self.bits = list(struct.unpack("<11I", raw("packing_bits_base64")))
        self.records = list(struct.iter_unpack("<HHI", raw("variant_items_base64")))
        self.spans = list(struct.iter_unpack("<HH", raw("variant_groups_base64")))
        if len(self.spans) != 11 or any(s+n > len(self.records) for s,n in self.spans) or any(b > 10 for b in self.bits):
            raise ValueError("invalid native Bot character groups")
        offsets = [0x18,0x1c,0x20,0x14,0x24,0x28,0x2c,0x44,0x48,0x4c,0x54,0x40,0x50,0x30,0x34,0x38,0x3c,0x58,0x5c,0x60,0x64]
        if list(struct.iter_unpack("<iii", raw("packing_fields_base64"))) != [(o,10 if o<0x40 else 6,0) for o in offsets]:
            raise ValueError("native Bot character field order mismatch")
        for key, expected in [
            ("selector_hashes_base64", [native_hash(n) for n in ["m","f","tf","sc","zm","wht","yel","blk","cau"]]),
            ("gender_selectors_base64", [2,3,4]),
            ("faction_selectors_base64", [0,1]),
            ("skin_selectors_base64", [5,6,7,8]),
        ]:
            if [v[0] for v in struct.iter_unpack("<I", raw(key))] != expected:
                raise ValueError("native Bot character selectors mismatch")
        self.variants = capture["variants"]

    def compose(self, items, gender, team, models):
        if gender not in ("m", "f") or team not in (1,2):
            raise ValueError("invalid Bot character gender/team")
        indices = [0]*11
        for group,item in items.items():
            g = GROUPS.index(group.lower())
            start,count = self.spans[g]
            matches = [r[0] for r in self.records[start:start+count] if r[2] == native_hash(item)]
            if len(matches) != 1 or not 0 < matches[0] <= 1 << self.bits[g] or self.bits[g] == 0:
                raise ValueError("Bot character item absent/ambiguous: " + item)
            indices[g] = matches[0]
        if not all(indices[:5]):
            raise ValueError("Bot character lacks mandatory groups")
        # A single native skin selector applies to all groups. Prefer the
        # composition closest to the prepared model list, but never combine
        # incompatible head/hand tones just because each has a table entry.
        candidates = []
        for skin in range(4):
            mask = (0x80000000 >> int(gender == "f")) | (0x80000000 >> (3 if team == 1 else 2)) | (0x80000000 >> (5+skin))
            selected = []
            for group,index in zip(GROUPS,indices):
                if index == 0: continue
                for variant in self.variants:
                    if variant["group"] != group or variant["index"] != index: continue
                    selectors = base64.b64decode(variant["selectors_base64"], validate=True)
                    if len(selectors) != 6: raise ValueError("invalid native Bot variant")
                    if struct.unpack_from("<I",selectors,2)[0] & mask == mask:
                        selected.append(variant["model"].lower());break
            if len(selected) == sum(i != 0 for i in indices):
                score = sum(n in models for n in selected)
                candidates.append((score,-skin,selected))
        if not candidates:
            raise ValueError("Bot outfit has no consistent native skin variant")
        _,negative_skin,selected = max(candidates)
        return dict(groups=indices,bits=self.bits,gender=int(gender=="f"),faction=int(team==1),skin=-negative_skin), selected


def prepare(state, capture, arsenal, stream_outfits=False, native_characters=None):
    for value in (state, capture, arsenal):
        if value["client_sha256"] != CLIENT_SHA256 or value["ncs_crc"] != state["ncs_crc"]:
            raise ValueError("Bot capture client/map identity mismatch")
    if capture.get("read_only") is not True or arsenal.get("read_only") is not True:
        raise ValueError("Bot preparation requires read-only captures")
    classes = rows(capture["tables"]["mp/botClassTable.csv"])
    records = {row[0]: row for row in classes if row[0]}
    if len(records) != len([row for row in classes if row[0]]):
        raise ValueError("duplicate Bot class field")
    weapons = {w["name"]: w for w in arsenal["weapons"]}
    available = {a["name"].lower() for a in capture["character_assets"] if a["kind"] == 4 and (stream_outfits or a["flags"] & 5)}
    models,registrations = model_registry(state,capture)
    next_model = max(models.values()) + 1
    fallbacks, roster = [], []

    def model_id(name):
        nonlocal next_model
        if name not in available:
            raise ValueError("character model is not mounted: " + name)
        if name not in models:
            if next_model >= 1024:
                raise ValueError("model registry exhausted")
            models[name] = next_model
            registrations.append((next_model, name))
            next_model += 1
        return models[name]

    def outfit(column, team):
        suffix, faction = ("_sc", "sc") if team == 1 else ("", "tf")
        gender = records["charGender" + suffix][column]
        names,items = [],{}
        for group in ("FPA", "FJA", "BOO", "HAN", "HEA", "HEG", "ELP", "KNP"):
            item = records["char" + group + suffix][column]
            if not item:
                continue
            table = capture["tables"]["mp/cocharitems/" + group.lower() + ".csv"]
            names.append(variant(table, item, gender, faction))
            items[group] = item
        if len(names) < 5:
            raise ValueError("native Bot outfit lacks mandatory body parts")
        character = None
        if native_characters:
            character,names = native_characters.compose(items,gender,team,names)
        return [model_id(n) for n in names],character

    for client in range(1, 12):
        column = 1 + (client - 1) % 9
        name = records["loadoutPrimary"][column].lower() + "_mp"
        weapon = weapons.get(name)
        if weapon is None:
            raise ValueError("native Bot class weapon not captured: " + name)
        for key, values in records.items():
            if key.startswith("loadoutPrimaryAttachment") and values[column] not in {"", "none"}:
                raise ValueError("native Bot class needs explicit attachment binding")
        definition = base64.b64decode(weapon["definition_base64"], validate=True)
        if len(definition) != 0xe28 or struct.unpack_from("<I", definition, 0x48c)[0] not in range(6):
            raise ValueError("native Bot class weapon firing mode unsupported")
        animation = animation_config(state["training_enemy"], capture, definition)
        outfits=[]
        for team in [1,2]:
            try:
                parts,character=outfit(column,team)
            except ValueError as error:
                if native_characters: raise
                fallbacks.append({"client":client,"team":team,"class":records["uniqueName"][column],"reason":str(error)})
                parts,character=outfit(1,team)
            outfits.append(dict(model=parts[0],attachment_models=parts[1:]+[0]*(11-len(parts))))
            if character: outfits[-1]["character"] = character
        team=state["match_config"].get("teams",[2]*6+[1]*6)[client]
        parts=[outfits[team-1]["model"]]+[i for i in outfits[team-1]["attachment_models"] if i]
        animation["model"] = parts[0]
        personality = records["botPersonalities"][column].split("|")[0]
        roster.append({"class": records["uniqueName"][column], "weapon_name": name, "animation": animation,
                       "attachment_models": parts[1:] + [0] * (11 - len(parts)), "personality": personality,"outfits":outfits})
    state["match_config"]["roster"] = roster
    state["match_config"]["models"] = registrations
    return {"classes": [b["class"] for b in roster], "weapons": [b["weapon_name"] for b in roster],
            "registered_models": registrations, "outfit_fallbacks": fallbacks}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-state", type=Path, required=True)
    parser.add_argument("--characters", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stream-outfits", action="store_true", help="allow captured DB placeholder names through the native asynchronous model loader")
    parser.add_argument("--native-characters", type=Path, help="read-only retained outfit cache for native sleeves/gloves in Bot replays")
    parser.add_argument("--rebind-animations", action="store_true", help="retain the current roster and bind each Bot's animations to its current weapon")
    args = parser.parse_args()
    state = json.loads(args.game_state.read_text())
    arsenal_path = args.game_state.parent / state["arsenal_file"]
    native = NativeBotCharacters(json.loads(args.native_characters.read_text())) if args.native_characters else None
    if args.rebind_animations:
        if args.stream_outfits or args.native_characters:
            parser.error("animation rebinding retains existing outfits")
        result = rebind_animations(state, json.loads(args.characters.read_text()), json.loads(arsenal_path.read_text()))
    else:
        result = prepare(state, json.loads(args.characters.read_text()), json.loads(arsenal_path.read_text()), args.stream_outfits, native)
    for name in ("arsenal_file", "collision_file"):
        state[name] = str((args.game_state.parent / state[name]).resolve())
    with args.output.open("x") as target:
        json.dump(state, target, indent=2)
        target.write("\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
