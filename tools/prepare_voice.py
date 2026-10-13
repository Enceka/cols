#!/usr/bin/env python3
"""Prepare native voice cue candidates from a CRC-verified 3.3.3.3 NCS capture.

The catalog records resource identities and transport selectors. Trigger timing,
recipient policy and audible playback need separate gameplay verification.
"""
import argparse
import base64
import hashlib
import json
import re
import struct
from pathlib import Path

from prepare_game_state import CLIENT_SHA256, compute, strings

CONSTANT_OFFSET = 512
MAX_SELECTOR = 16000
ANNOUNCER = {
    "mode_tdm": "tm_deathmatch",
    "match_start": "boost",
    "victory": "mission_success",
    "defeat": "mission_fail",
    "draw": "draw",
    "lead_taken": "lead_taken",
    "lead_lost": "lead_lost",
    "lead_tied": "tied",
    "time_warning_winning": "winning",
    "time_warning_losing": "losing",
    "score_warning_winning": "winning_fight",
    "score_warning_losing": "losing_fight",
    "time_up": "timesup",
    "halftime": "halftime",
    "overtime": "overtime",
}
CHARACTER = {
    "reload": "inform_reloading_generic",
    "frag_out": "inform_attack_grenade",
    "flash_out": "inform_attack_flashbang",
    "smoke_out": "inform_attack_smoke",
    "stun_out": "inform_attack_stun",
    "c4_out": "inform_attack_thwc4",
    "claymore_plant": "inform_plant_claymore",
    "kill_confirmed": "inform_killfirm_infantry",
    "pain": "mp_pain",
    "death": "mp_death",
    "sprint": "mp_sprint",
}


def constant_bank(capture, category):
    """Match native source/name ordering and dedup, then check loaded order."""
    previous = set()
    values = []
    prefix = capture["localized_prefix"].encode("ascii").lower()
    assets = [a for a in capture["assets"] if a.get("category") == category]
    ordered = sorted(assets, key=lambda a: (
        a["source"], len(a["name"]) > 12 and
        a["name"][8:].encode("ascii").lower().startswith(prefix), a["name"].lower()))
    for asset in ordered:
        current = strings(asset["strings_base64"])
        values.extend(v for v in current if v not in previous)
        previous.update(current)
    linked = []
    for asset in capture["linked"]:
        if asset["category"] == category:
            current = strings(asset["strings_base64"])
            if len(current) != asset["count"]:
                raise ValueError("linked sound resource count mismatch")
            linked.extend(current)
    if values != linked:
        raise ValueError("voice constant order differs from initialized client")
    names = [v.decode("ascii") for v in values]
    if len(names) != len(set(n.lower() for n in names)):
        raise ValueError("ambiguous case-insensitive sound aliases")
    if any(not re.fullmatch(r"[A-Za-z0-9_.,/\\-]{1,255}", n) for n in names):
        raise ValueError("invalid sound alias name")
    return names


def prepare(ncs_bytes, prepared_bytes, voice_bytes=None):
    capture = json.loads(ncs_bytes)
    state = json.loads(prepared_bytes)
    if capture.get("initialized") != 1 or capture.get("connection_state") not in (9, 10):
        raise ValueError("voice preparation requires initialized map resources")
    if state.get("client_sha256") != CLIENT_SHA256 or state.get("schema_version") != 1:
        raise ValueError("prepared map client identity mismatch")
    if not re.fullmatch(r"(?:mp|za)_[a-z0-9_]+", state.get("map", "")):
        raise ValueError("invalid prepared map name")
    verified = compute(capture)
    if verified["ncs_crc"] != state["ncs_crc"]:
        raise ValueError("voice resources belong to another prepared map CRC")
    layout = base64.b64decode(capture["category_layout_base64"], validate=True)
    if len(layout) != 21 * 12 or any(
            struct.unpack_from("<III", layout, c * 12) != expected
            for c, expected in [(5, (7058, 0, 0)), (6, (3107, 512, 1))]):
        raise ValueError("native sound category layout mismatch")
    constants = constant_bank(capture, 5)
    dynamics = constant_bank(capture, 6)
    if not constants or len(constants) > MAX_SELECTOR - CONSTANT_OFFSET + 1 or len(dynamics) >= 512:
        raise ValueError("native sound selector bank exceeds range")
    sources = [hashlib.sha256(b).hexdigest() for b in (ncs_bytes, prepared_bytes)]
    loaded = {}
    if voice_bytes is not None:
        voice = json.loads(voice_bytes)
        if (voice.get("schema_version") != 1 or voice.get("read_only") is not True or
                voice.get("client_sha256") != CLIENT_SHA256 or
                voice.get("connection_state") != 10 or voice.get("ncs_crc") != state["ncs_crc"] or
                voice.get("map") != state["map"]):
            raise ValueError("voice alias capture identity/state mismatch")
        for alias in voice["aliases"]:
            name = alias["name"].lower()
            if name in loaded or "error" in alias:
                raise ValueError("ambiguous or incomplete voice alias capture")
            loaded[name] = alias
        sources.append(hashlib.sha256(voice_bytes).hexdigest())

    lookup = {}
    for index, name in enumerate(constants):
        lookup[name.lower()] = dict(alias=name, selector=CONSTANT_OFFSET + index,
                                   bank="constant", constant_index=index)
    for index, name in enumerate(dynamics, 1):
        lookup.setdefault(name.lower(), dict(alias=name, selector=index,
                                             bank="dynamic", configstring=3107 + index))

    def cue(name):
        sound = lookup.get(name.lower())
        if sound is None:
            return dict(alias=name, available=False)
        record = dict(sound, available=True, play_command="p " + str(sound["selector"]))
        resource = loaded.get(name.lower())
        record["resource_status"] = ("loaded_alias" if resource is not None and
                                     resource["entry_flags"] & 5 and resource.get("count", 0) > 0
                                     else "constant_only")
        if resource is not None:
            record["variant_count"] = resource.get("count", 0)
        return record

    announcers = {faction: {event: cue(prefix + "_1mc_" + suffix)
                           for event, suffix in ANNOUNCER.items()}
                  for faction, prefix in [("tf", "uk"), ("sc", "ab")]}
    music = {faction: {event: cue(prefix + "_" + event + "_music")
                      for event in ("spawn", "victory", "defeat", "winning", "losing")}
             for faction, prefix in [("tf", "uk"), ("sc", "ab")]}
    characters = {}
    for faction in ("uk", "ab"):
        for gender in ("m", "f"):
            for variant in (1, 2):
                prefix = f"{faction}_{gender}{variant}_"
                characters[prefix] = dict(
                    prefix_command=f's cg_vo_prefix "{prefix.upper()}"',
                    events={event: cue(prefix + suffix) for event, suffix in CHARACTER.items()},
                    aliases=[cue(n) for n in constants if n.lower().startswith(prefix)])
    return dict(schema_version=1, client_sha256=CLIENT_SHA256, sources_sha256=sources,
                map=state["map"], mode=state["mode"], ncs_crc=verified["ncs_crc"],
                constant_sound_count=len(constants), dynamic_sound_count=len(dynamics),
                evidence="CRC-verified native resource identities; trigger and audio acceptance pending",
                announcers=announcers, music=music, characters=characters,
                countdown=cue("ui_mp_timer_countdown"),
                all_constant_sounds=[dict(alias=n, selector=CONSTANT_OFFSET+i)
                                     for i, n in enumerate(constants)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path, help="initialized --net-constants capture")
    parser.add_argument("--game-state", type=Path, required=True, help="prepared map to bind by CRC")
    parser.add_argument("--voice-assets", type=Path, help="optional same-map --voice-assets capture")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = prepare(args.capture.read_bytes(), args.game_state.read_bytes(),
                     args.voice_assets.read_bytes() if args.voice_assets else None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(dict(output=str(args.output), map=result["map"], ncs_crc=result["ncs_crc"],
                          constant_sound_count=result["constant_sound_count"],
                          character_families=len(result["characters"]))))


if __name__ == "__main__":
    main()
