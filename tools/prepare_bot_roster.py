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

from prepare_bot_locomotion import blocks, rawfile, select_action
from prepare_game_state import CLIENT_SHA256


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
    for a in capture["animations"]:
        if a.get("source", 0x07817b80) == 0x07817b80:
            indices.setdefault(a["name"].lower(), set()).add(a["index"])

    def index(name):
        found = indices.get(name.lower(), set())
        if len(found) != 1 or not 0 < next(iter(found)) < 1024:
            raise ValueError("native Bot animation missing or ambiguous: " + name)
        return next(iter(found))

    config = copy.deepcopy(template)
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
    fire = select_action(script, "fireweapon", context)
    if fire is None or fire[0] != "torso" or select_action(script, "fireweapon", dict(context, movetype="walk")) is not None:
        raise ValueError("unsupported native Bot firing overlay")
    locomotion["fire_standing"] = index(fire[1])
    reload = select_action(script, "reload", context)
    if (reload is None or reload[0] != "torso"
            or reload != select_action(script, "reload", dict(context, movetype="walk"))):
        raise ValueError("unsupported native Bot reload overlay")
    config["reload_animation"] = index(reload[1])
    locomotion["script_sha256"] = hashlib.sha256(script_bytes).hexdigest()
    config["locomotion"] = locomotion
    return config


def prepare(state, capture, arsenal, stream_outfits=False):
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
    models = {m["name"].lower(): m["model"] for m in capture["models"]}
    next_model = max(models.values()) + 1
    registrations, fallbacks, roster = [], [], []

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
        names = []
        for group in ("FPA", "FJA", "BOO", "HAN", "HEA", "HEG", "ELP", "KNP"):
            item = records["char" + group + suffix][column]
            if not item:
                continue
            table = capture["tables"]["mp/cocharitems/" + group.lower() + ".csv"]
            names.append(variant(table, item, gender, faction))
        if len(names) < 5:
            raise ValueError("native Bot outfit lacks mandatory body parts")
        return [model_id(n) for n in names]

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
        try:
            parts = outfit(column, 2 if client <= 5 else 1)
        except ValueError as error:
            fallbacks.append({"client": client, "class": records["uniqueName"][column], "reason": str(error)})
            parts = outfit(1, 2 if client <= 5 else 1)
        animation["model"] = parts[0]
        personality = records["botPersonalities"][column].split("|")[0]
        roster.append({"class": records["uniqueName"][column], "weapon_name": name, "animation": animation,
                       "attachment_models": parts[1:] + [0] * (11 - len(parts)), "personality": personality})
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
    args = parser.parse_args()
    state = json.loads(args.game_state.read_text())
    arsenal_path = args.game_state.parent / state["arsenal_file"]
    result = prepare(state, json.loads(args.characters.read_text()), json.loads(arsenal_path.read_text()), args.stream_outfits)
    for name in ("arsenal_file", "collision_file"):
        state[name] = str((args.game_state.parent / state[name]).resolve())
    with args.output.open("x") as target:
        json.dump(state, target, indent=2)
        target.write("\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
