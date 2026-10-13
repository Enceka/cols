#!/usr/bin/env python3
"""Build/run a read-only Win32 state probe inside the project's GameHub prefix.

No DebugActiveProcess, memory writes or injection. Output is a small allowlist
of known client state fields; no account/session keys are read.
"""
import argparse
import fcntl
import json
import os
import re
from pathlib import Path
import subprocess
import time

from clientlib import REPO, sha256
from gamehub_run import engines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--build", action="store_true")
    mode.add_argument("--experiment", type=Path, help="a running experiment.json")
    parser.add_argument("--prefix-name", help="project-owned build prefix; experiment selects its own prefix")
    parser.add_argument("--sample-ready", type=Path, help="new experiment-local readiness receipt for replay sampling")
    parser.add_argument("--weapon-models-target", choices=("m4_mp","famasss_mp","ak104ss_mp"),
                        help="fixed weapon model/animation target; default M4")
    extract = parser.add_mutually_exclusive_group()
    extract.add_argument("--frontend-assets", action="store_true", help="read the fixed native frontend movie and map/mode tables")
    extract.add_argument("--assets", action="store_true", help="extract the fixed profile-schema and data-table allowlist")
    extract.add_argument("--profiles", action="store_true", help="save this client's two fixed profile buffers to the ignored experiment directory")
    extract.add_argument("--net-constants", action="store_true", help="read loaded type-44 net-constant assets and CRC state")
    extract.add_argument("--map-entities", action="store_true", help="read loaded type-19 map entity strings")
    extract.add_argument("--bootstrap-weapon", action="store_true", help="read the registered basic M4 and ammo keys")
    extract.add_argument("--weapon-models", action="store_true", help="read M4 model tables and composed model parts")
    extract.add_argument("--arsenal", action="store_true", help="read registered weapon definitions and attachment attributes")
    extract.add_argument("--collision", action="store_true", help="read training map collision brushes and terrain")
    extract.add_argument("--glass",action="store_true",help="read this map's native glass piece identities and state")
    extract.add_argument("--weapon-registry", action="store_true", help="read current weapon name/index identities, including the lobby")
    extract.add_argument("--training-target", action="store_true", help="read network target model, measured bounds and entity19 DObj")
    extract.add_argument("--bullet-impacts", action="store_true", help="read current impact entities, one-shot latches and loaded impact FX")
    extract.add_argument("--characters", action="store_true", help="read registered character model bones, tables and client assembly state")
    extract.add_argument("--map-check", action="store_true", help="read only current map identity, actor models and its compass texture")
    extract.add_argument("--hit-regions", action="store_true", help="read the training enemy's current bone hit regions; the bot must stand still")
    extract.add_argument("--feedback", action="store_true", help="sample native hit HUD and fixed feedback assets read-only for five seconds")
    extract.add_argument("--replay", action="store_true", help="sample native killcam/player state read-only for ten seconds")
    extract.add_argument("--grounding", action="store_true", help="sample current calculated Bot feet read-only for ten seconds")
    extract.add_argument("--outfits", action="store_true", help="read retained character item and variant caches")
    extract.add_argument("--appearance-load", action="store_true", help="sample native actor models and loader state across map startup for twenty-five seconds")
    extract.add_argument("--tactical-assets", action="store_true", help="read native shock files, status tables, NCS order and parameter blocks")
    extract.add_argument("--hud-assets", action="store_true", help="read loaded native HUD/menu identities and fixed HUD rawfiles")
    extract.add_argument("--voice-assets", action="store_true", help="read loaded native announcer/character voice aliases and related tables")
    extract.add_argument("--voice-playback", action="store_true", help="sample native voice channels and one-shot sound entities read-only for twenty seconds")
    extract.add_argument("--nuke-assets", action="store_true", help="read the two fixed registered nuclear reward weapons")
    extract.add_argument("--nuke-state", action="store_true", help="sample native streak progress, action slots and inventory read-only")
    extract.add_argument("--nuke-frame", action="store_true", help="read one native nuclear HUD/inventory and actor frame")
    args = parser.parse_args()
    if args.weapon_models_target and (not args.weapon_models or not args.experiment):
        parser.error("weapon-models-target requires weapon-models and a running experiment")
    if args.sample_ready and (not args.experiment or not (args.replay or args.voice_playback)):
        parser.error("sample-ready requires an experiment and replay or voice playback sampling")
    if args.sample_ready:
        args.sample_ready = args.sample_ready.resolve()
        if args.sample_ready.parent != args.experiment.resolve().parent or args.sample_ready.exists():
            parser.error("sample-ready must be a new file in this experiment directory")
    if args.prefix_name and (not args.build or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", args.prefix_name)):
        parser.error("prefix-name requires build and a simple project directory name")
    if args.build and (args.nuke_assets or args.nuke_state or args.nuke_frame or args.voice_playback or args.voice_assets or args.hud_assets or args.frontend_assets or args.glass or args.map_check or args.tactical_assets or args.appearance_load or args.outfits or args.grounding or args.replay or args.feedback or args.assets or args.profiles or args.net_constants or args.map_entities or args.bootstrap_weapon or args.weapon_models or args.arsenal or args.collision or args.weapon_registry or args.training_target or args.bullet_impacts or args.characters or args.hit_regions):
        parser.error("extraction options require a running experiment")
    state = json.loads(args.experiment.read_text()) if args.experiment else None
    engine_id = state["engine_id"] if state else "10000033"
    engine = Path(engines()[engine_id]["install_path"])
    prefix = Path(state["prefix"]) if state else REPO / "client-work" / (args.prefix_name or "wine10-prefix")
    if not prefix.resolve().is_relative_to((REPO / "client-work").resolve()):
        parser.error("prefix must belong to this project")
    marker = prefix / ".codol-prefix.json"
    if not marker.is_file() or json.loads(marker.read_text())["engine_id"] != engine_id:
        parser.error("prefix ownership marker mismatch")
    sources = [REPO / "tools/ReadClientState.cs", REPO / "tools/ClientAssets.cs", REPO / "tools/HitRegions.cs", REPO / "tools/ClientVoice.cs", REPO / "tools/ClientNuke.cs"]
    source_hashes = {p.name: sha256(p) for p in sources}
    binary = REPO / ".tools/ReadClientState.exe"
    stamp = binary.with_suffix(".build.json")
    env = os.environ.copy()
    env.update(WINEPREFIX=str(prefix.resolve()), WINEDEBUG="-all")
    wine = str(engine / "bin/wine")
    if args.build:
        binary.parent.mkdir(exist_ok=True)
        with (prefix / ".codol-lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                built = subprocess.run([
                    wine, r"C:\windows\Microsoft.NET\Framework\v4.0.30319\csc.exe",
                    "/nologo", "/platform:anycpu", "/r:System.Web.Extensions.dll", "/out:Z:" + str(binary), *["Z:" + str(p) for p in sources],
                ], env=env, capture_output=True, text=True, timeout=30)
                if built.returncode:
                    raise RuntimeError((built.stdout + built.stderr)[-2000:])
                stamp.write_text(json.dumps({"sources_sha256": source_hashes, "binary_sha256": sha256(binary)}) + "\n")
                print(json.dumps({"built": str(binary), "read_only": True}))
            finally:
                # A standalone compiler can let Wine exit before cleanup.
                # -k then returns 1 with no output; -w confirms it is stopped.
                stopped = subprocess.run([str(engine / "bin/wineserver"), "-k"], env=env,
                                         capture_output=True, text=True, timeout=15)
                if stopped.returncode not in (0, 1) or (stopped.returncode == 1 and stopped.stderr):
                    stopped.check_returncode()
                subprocess.run([str(engine / "bin/wineserver"), "-w"], env=env,
                               capture_output=True, timeout=15, check=True)
    else:
        if "finished_unix" in state:
            parser.error("experiment has already ended")
        actual = subprocess.check_output(["/bin/ps", "-p", str(state["pid"]), "-o", "comm="], text=True).strip()
        from clientlib import matches_client_image
        if not matches_client_image(actual, state["argv"][1], prefix):
            parser.error("recorded PID is no longer the research client")
        if not stamp.is_file():
            parser.error("build the probe first with --build")
        hashes = json.loads(stamp.read_text())
        if hashes != {"sources_sha256": source_hashes, "binary_sha256": sha256(binary)}:
            parser.error("probe changed; rebuild it")
        command = [wine, str(binary)] + (["--nuke-assets"] if args.nuke_assets else ["--nuke-state"] if args.nuke_state else ["--nuke-frame"] if args.nuke_frame else ["--voice-playback"] if args.voice_playback else ["--voice-assets"] if args.voice_assets else ["--frontend-assets"] if args.frontend_assets else ["--glass"] if args.glass else ["--map-check"] if args.map_check else ["--hud-assets"] if args.hud_assets else ["--tactical-assets"] if args.tactical_assets else ["--appearance-load"] if args.appearance_load else ["--outfits"] if args.outfits else ["--grounding"] if args.grounding else ["--replay"] if args.replay else ["--feedback"] if args.feedback else ["--assets"] if args.assets else ["--profiles"] if args.profiles else ["--net-constants"] if args.net_constants else ["--map-entities"] if args.map_entities else ["--bootstrap-weapon"] if args.bootstrap_weapon else ["--weapon-models"] if args.weapon_models else ["--arsenal"] if args.arsenal else ["--collision"] if args.collision else ["--weapon-registry"] if args.weapon_registry else ["--training-target"] if args.training_target else ["--bullet-impacts"] if args.bullet_impacts else ["--characters"] if args.characters else ["--hit-regions"] if args.hit_regions else [])
        if args.weapon_models_target:
            command.append(args.weapon_models_target)
        if args.sample_ready:
            # Consume the native marker while stdout remains file-backed. This
            # avoids both startup races and pipe deadlocks from large frames.
            import tempfile
            with tempfile.TemporaryFile(mode="w+") as output_stream:
                with subprocess.Popen(command, env=env, stdout=output_stream,
                                      stderr=subprocess.PIPE, text=True) as probe:
                    import threading
                    timer = threading.Timer(30, probe.kill)
                    timer.start()
                    try:
                        errors = []
                        for line in probe.stderr:
                            errors.append(line)
                            if line.strip() == ("sample_ready=voice-playback" if args.voice_playback else "sample_ready=replay"):
                                args.sample_ready.write_text(json.dumps(dict(mode="voice-playback" if args.voice_playback else "replay", read_only=True)) + "\n")
                        status = probe.wait()
                    finally:
                        timer.cancel()
                output_stream.seek(0)
                result = subprocess.CompletedProcess(command, status, output_stream.read(), "".join(errors))
        else:
            result = subprocess.run(command, env=env, capture_output=True, text=True,
                                    timeout=40 if args.appearance_load else 30)
        if result.returncode:
            raise RuntimeError((result.stdout + result.stderr)[-2000:])
        decoded = json.loads(result.stdout)
        output = args.experiment.parent / (("nuke-assets-" if args.nuke_assets else "nuke-state-" if args.nuke_state else "nuke-frame-" if args.nuke_frame else "voice-playback-" if args.voice_playback else "voice-assets-" if args.voice_assets else "frontend-assets-" if args.frontend_assets else "glass-" if args.glass else "map-check-" if args.map_check else "hud-assets-" if args.hud_assets else "tactical-assets-" if args.tactical_assets else "appearance-load-" if args.appearance_load else "outfits-" if args.outfits else "grounding-" if args.grounding else "replay-" if args.replay else "feedback-" if args.feedback else "client-assets-" if args.assets else "client-profiles-" if args.profiles else "net-constants-" if args.net_constants else "map-entities-" if args.map_entities else "bootstrap-weapon-" if args.bootstrap_weapon else "weapon-models-" if args.weapon_models else "arsenal-" if args.arsenal else "collision-" if args.collision else "weapon-registry-" if args.weapon_registry else "training-target-" if args.training_target else "bullet-impacts-" if args.bullet_impacts else "characters-" if args.characters else "hit-regions-" if args.hit_regions else "client-state-") + str(int(time.time() * 1000)) + ".json")
        output.write_text(json.dumps(decoded, indent=2) + "\n")
        summary = {"schemas": {k: {f: v[f] for f in ("found", "error") if f in v} for k, v in decoded["schemas"].items()},
                   "tables": {k: {f: v[f] for f in ("found", "rows", "columns", "error") if f in v} for k, v in decoded["tables"].items()}} if args.assets else {"read_only": True, "profile_buffers": 2} if args.profiles else decoded
        if args.frontend_assets:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "rawfiles": {k: {f: v[f] for f in ("found", "length", "compressed_length") if f in v} for k, v in decoded["rawfiles"].items()}}
        if args.voice_assets:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "ncs_crc": decoded["ncs_crc"],
                       "sound_asset_count": decoded["sound_asset_count"], "voice_alias_count": len(decoded["aliases"]),
                       "alias_errors": sum("error" in a for a in decoded["aliases"]),
                       "rawfiles": len(decoded["rawfiles"]), "tables": len(decoded["tables"])}
        if args.hud_assets:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "menus": len(decoded["menus"])}
        elif args.tactical_assets:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "rawfiles": len(decoded["rawfiles"]), "tables": len(decoded["tables"]), "shocks": len(decoded["shocks"])}
        if args.net_constants:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "initialized": decoded["initialized"], "checksum": decoded["checksum"], "assets": len(decoded["assets"]), "linked": len(decoded["linked"]), "asset_errors": sum("error" in asset for asset in decoded["assets"])}
        if args.map_entities:
            summary = {"read_only": True, "connection_state": decoded["connection_state"], "maps": [{"name":m["name"],"byte_count":m["byte_count"]} for m in decoded["maps"]]}
        if args.arsenal or args.collision or args.weapon_registry:
            summary={"read_only":True,"weapons":len(decoded.get("weapons",[])),"brushes":len(decoded.get("brushes",[])),"triangles":len(decoded.get("triangles",[]))}
        if args.glass:
            summary = {"read_only": True, "map": decoded["map"], "piece_count": decoded["piece_count"]}
        if args.outfits:
            summary = {"read_only": True, "items": len(decoded["items"]), "variants": len(decoded["variants"])}
        if args.nuke_assets:
            summary = {"read_only": True, "weapons": len(decoded["weapons"]), "effects": len(decoded["effects"])}
        if args.nuke_state or args.nuke_frame or args.voice_playback or args.replay or args.grounding or args.appearance_load:
            summary = {"read_only": True, "frames": len(decoded["frames"])}
        if args.feedback:
            summary = {"read_only": True, "ncs_crc": decoded["ncs_crc"], "frames": len(decoded["frames"]),
                       "resources": {k: {f: v[f] for f in ("found", "error") if f in v} for k, v in decoded["resources"].items()}}
        if args.hit_regions:
            summary = {"read_only": True, "bone_count": decoded["bone_count"], "capture_origin": decoded["capture_origin"]}
        print(json.dumps({"output": str(output), "state": summary}))


if __name__ == "__main__":
    main()
