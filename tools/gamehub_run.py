#!/usr/bin/env python3
"""Run bounded CODOL experiments with an installed GameHub engine.

Uses only project-owned prefixes and a marked research client copy. GameHub's
existing containers and installation metadata are never modified.
"""
import argparse
import fcntl
import json
import os
import re
import signal
import shutil
import subprocess
import time
from pathlib import Path

from clientlib import DEFAULT_WORKSPACE, REPO, checked_sample, manifest, sha256

ENGINE_INDEX = Path.home() / "Library/Application Support/com.gamemac.www/wine-engine/container/wine_installations.json"
LAB_MARKER = ".codol-research-copy.json"


def engines():
    data = json.loads(ENGINE_INDEX.read_text())
    return data["wine_installations"]


def stop_prefix(engine, env):
    subprocess.run([str(engine / "bin/wineserver"), "-k"], env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=True)


def trust_lab_certificate(work, certificate, output, backups):
    """IIPS can change the process CWD before curl opens relative cacert.pem."""
    original = (work / "cacert.pem").read_bytes()
    trusted = original + b"\n" + certificate.read_bytes()
    for relative, backup_name in [("cacert.pem", "cacert.before.pem"), ("IIPS/cacert.pem", "iips-cacert.before.pem")]:
        path = work / relative
        previous = path.read_bytes() if path.exists() else None
        backups[path] = previous
        if previous is not None:
            (output / backup_name).write_bytes(previous)
        path.write_bytes(trusted)


def restore_certificates(backups):
    for path, previous in backups.items():
        if previous is None:
            path.unlink(missing_ok=True)
        else:
            path.write_bytes(previous)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["list", "prepare", "init", "probe", "client"])
    parser.add_argument("--engine-id", default="10000033")
    parser.add_argument("--prefix-name", help="named project-owned prefix for isolated diagnostics; never resets an existing prefix")
    parser.add_argument("--seconds", type=int, default=90)
    parser.add_argument("--title-id-hex", "--context-hex", dest="context_hex", help="explicit u32 title_id; valid game deployment value is still unknown")
    parser.add_argument("--file-trace", action="store_true")
    parser.add_argument("--crossproxy", action="store_true", help="also reproduce the launcher's CrossProxy step")
    parser.add_argument("--tls-probe", action="store_true", help="use a loopback-only OpenSSL TLS 1.0 research listener for endpoint B")
    parser.add_argument("--rust-auth", action="store_true", help="use the in-process Rust AUTH TLS/HTTP endpoint")
    parser.add_argument("--release-server", action="store_true", help="run the optimized Rust server built with cargo build --release")
    parser.add_argument("--auth-reply-file", type=Path, help="experimental JSON reply for --rust-auth")
    parser.add_argument("--local-auth-tickets", action="store_true", help="ask the Rust endpoint to issue local test-account tickets")
    parser.add_argument("--rust-lobby", action="store_true", help="enable the Rust protocol-210 lobby handshake observer")
    parser.add_argument("--rust-stun", action="store_true", help="enable loopback UDP discovery on 3074/3078")
    parser.add_argument("--rust-game", action="store_true", help="enable experimental game allocation/UDP transport; no simulation yet")
    parser.add_argument("--game-state", type=Path, help="opt-in prepared training gamestate from prepare_game_state.py")
    parser.add_argument("--game-map-catalog", type=Path, help="captured preparations for native room map selection")
    parser.add_argument("--snapshot-probe", nargs="?", type=int, const=60, help="bootstrap frame budget at 20 Hz (1..18000; default 60); requires game-state")
    parser.add_argument("--hit-regions", type=Path, help="enemy pose capture path; may be written later by client_state.py --hit-regions")
    parser.add_argument("--local-services", action="store_true", help="enable time/storage services with persistent data in client-work/server-data")
    parser.add_argument("--service-data-dir", type=Path, help="explicit isolated service-data directory for diagnostics; requires --local-services")
    parser.add_argument("--quiet-network", action="store_true", help="omit verbose Wine winsock tracing during longer UI tests; Rust captures remain enabled")
    parser.add_argument("--trust-lab-cert", action="store_true", help="temporarily add the lab certificate to ONLY the research copy's cacert.pem")
    args = parser.parse_args()
    available = engines()
    if args.action == "list":
        print(json.dumps([{k: info[k] for k in ["id", "name", "version", "install_path", "install_status"]}
                          for info in available.values()], indent=2))
        return 0
    if not 1 <= args.seconds <= 900:
        parser.error("seconds must be 1..900")
    if args.prefix_name and not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", args.prefix_name):
        parser.error("prefix-name must be a simple directory name, at most 64 characters")
    if (args.tls_probe or args.rust_auth or args.trust_lab_cert) and args.action != "client":
        parser.error("TLS options require a client experiment")
    if args.tls_probe and args.rust_auth:
        parser.error("select one AUTH backend")
    if args.auth_reply_file and not args.rust_auth:
        parser.error("auth-reply-file requires rust-auth")
    if args.local_auth_tickets and (not args.rust_auth or args.auth_reply_file):
        parser.error("local-auth-tickets requires rust-auth without a reply fixture")
    if args.rust_lobby and not args.local_auth_tickets:
        parser.error("rust-lobby requires local-auth-tickets")
    if args.rust_stun and not args.rust_lobby:
        parser.error("rust-stun requires rust-lobby")
    if args.local_services and not args.rust_lobby:
        parser.error("local-services requires rust-lobby")
    if args.service_data_dir and not args.local_services:
        parser.error("service-data-dir requires local-services")
    if args.rust_game and not args.local_services:
        parser.error("rust-game requires local-services")
    if args.game_state and (not args.rust_game or not args.game_state.is_file()):
        parser.error("game-state requires rust-game and an existing prepared file")
    if args.snapshot_probe is not None and (not args.game_state or not 1 <= args.snapshot_probe <= 18000):
        parser.error("snapshot-probe requires game-state and 1..18000 frames")
    if args.game_map_catalog and (not args.rust_game or not args.game_map_catalog.is_file()):
        parser.error("game-map-catalog requires rust-game and an existing catalog")
    if args.release_server and not args.rust_auth:
        parser.error("release-server requires rust-auth")
    if args.hit_regions and not args.game_state:
        parser.error("hit-regions requires game-state")
    if args.trust_lab_cert and not (args.tls_probe or args.rust_auth):
        parser.error("trust-lab-cert requires tls-probe or rust-auth")
    info = available[args.engine_id]
    engine = Path(info["install_path"])
    if info["install_status"] != "completed" or not (engine / "bin/wine").is_file():
        parser.error("selected GameHub engine is not installed")
    lab = REPO / "client-work"
    lab.mkdir(exist_ok=True)
    if args.action == "prepare":
        work = lab / "CODOL"
        if work.exists():
            parser.error("research copy already exists; refusing to merge or overwrite it")
        checked_sample(DEFAULT_WORKSPACE, "CODOL/codoMP_client_shipRetail.exe")
        subprocess.run(["cp", "-cR", str(DEFAULT_WORKSPACE / "CODOL"), str(work)], check=True)
        (work / LAB_MARKER).write_text(json.dumps({"source": str(DEFAULT_WORKSPACE / "CODOL"), "method": "APFS clone via cp -cR", "purpose": "CODOL research; separate writable copy"}) + "\n")
        print(json.dumps({"prepared": str(work)}))
        return 0
    prefix = lab / (args.prefix_name or ("wine10-prefix" if args.engine_id == "10000033" else "gamehub-" + args.engine_id))
    prefix.mkdir(exist_ok=True)
    marker = prefix / ".codol-prefix.json"
    if marker.exists():
        if json.loads(marker.read_text())["engine_id"] != args.engine_id:
            parser.error("prefix belongs to another engine")
    elif any(p.name != ".codol-prefix.json" for p in prefix.iterdir()):
        parser.error("existing prefix lacks project ownership marker")
    else:
        marker.write_text(json.dumps({"engine_id": args.engine_id, "purpose": "CODOL research"}) + "\n")
    lock = (prefix / ".codol-lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    work = lab / "CODOL"
    if args.action == "client":
        if not (work / LAB_MARKER).is_file():
            parser.error("create and mark a separate research client copy first; see docs/GAMEHUB_DEBUG.md")
        expected = next(f for f in manifest()["files"] if f["path"] == "CODOL/codoMP_client_shipRetail.exe")
        if sha256(work / "codoMP_client_shipRetail.exe") != expected["sha256"]:
            parser.error("client executable differs from the recorded sample")
        if args.context_hex is None:
            parser.error("client experiments require an explicit --title-id-hex")
        value = args.context_hex.removeprefix("0x").removeprefix("0X")
        if not value or len(value) > 8 or any(c not in "0123456789abcdefABCDEF" for c in value):
            parser.error("title-id-hex must be a u32 hexadecimal value")
        context = int(value, 16)
    else:
        context = None
    output = REPO / "research-output/gamehub" / (args.action + "-" + str(int(time.time() * 1000)))
    output.mkdir(parents=True)
    env = os.environ.copy()
    channels = "-all,err+all,+timestamp,+pid,+tid,+seh,+loaddll,+winsock,+debugstr"
    if args.quiet_network:
        channels = channels.replace(",+winsock", "")
    if args.file_trace:
        channels += ",+file"
    env.update(WINEPREFIX=str(prefix), WINEDEBUG=channels, WINEDLLOVERRIDES="winemenubuilder.exe=d")
    wine = str(engine / "bin/wine")
    server = process = cross = tls = None
    ca_backups = {}
    state = {"engine_id": args.engine_id, "engine": info["name"], "engine_version": info["version"],
             "prefix": str(prefix), "action": args.action, "title_id_experimental": context,
             "wine_debug": channels, "seconds": args.seconds, "started_unix": time.time()}
    with (output / "wine.log").open("wb") as log, (output / "observer.log").open("wb") as server_log, (output / "crossproxy.log").open("wb") as cross_log, (output / "tls-plaintext.bin").open("wb") as tls_plaintext, (output / "tls-status.log").open("wb") as tls_status:
        try:
            # Terminating the harness must still restore temporary certificate
            # trust and stop its own children. SIGKILL cannot run this cleanup.
            def interrupted(signum, _frame):
                raise KeyboardInterrupt("signal " + str(signum))
            signal.signal(signal.SIGTERM, interrupted)
            if args.action == "client":
                bootstrap = "127.0.0.1 39001\n127.0.0.1 39002\n0x%08X\n" % context
                (work / "dwauth.ini").write_text(bootstrap, encoding="ascii")
                (work / "stun.ini").write_text("127.0.0.1\n", encoding="ascii")
                (work / "iips.ini").write_text('enable_iips=1\niips_url="http://127.0.0.1:39003/"\n', encoding="ascii")
                state["client_sha256"] = expected["sha256"]
                state["crossproxy_enabled"] = args.crossproxy
                state["generated_configs"] = {name: (work / name).read_text() for name in ["dwauth.ini", "stun.ini", "iips.ini"]}
                cfg = (REPO / "configs/local.toml").read_text().replace("../captures", str(output / "captures"))
                if args.rust_lobby:
                    parts=cfg.split("\n[[listeners]]\n")
                    cfg=parts[0]+"".join("\n[[listeners]]\n"+part for part in parts[1:] if 'name = "endpoint_a"' not in part)
                state["tls_probe"] = args.tls_probe
                state["rust_auth"] = args.rust_auth
                state["lab_certificate_temporarily_trusted"] = args.trust_lab_cert
                if args.tls_probe or args.rust_auth:
                    parts = cfg.split("\n[[listeners]]\n")
                    cfg = parts[0] + "".join("\n[[listeners]]\n" + part for part in parts[1:] if 'name = "endpoint_b"' not in part)
                    certificate = REPO / ".tools/tls-lab/server.pem"
                    key = REPO / ".tools/tls-lab/key.pem"
                    if not certificate.is_file() or not key.is_file():
                        raise RuntimeError("create the local TLS lab certificate first; see GAMEHUB_DEBUG.md")
                    if args.rust_auth:
                        executable = REPO / "target" / ("release" if args.release_server else "debug") / "codol-server"
                        tls_argv = [str(executable), "serve-auth", "--cert", str(certificate), "--key", str(key), "--capture-dir", str(output / "auth-captures")]
                        if args.auth_reply_file:
                            tls_argv += ["--reply-file", str(args.auth_reply_file.resolve())]
                            state["auth_reply_sha256"] = sha256(args.auth_reply_file)
                        if args.local_auth_tickets:
                            tls_argv += ["--local-title-id", str(context)]
                        if args.rust_lobby:
                            tls_argv += ["--lobby-bind", "127.0.0.1:39001"]
                        if args.local_services:
                            tls_argv += ["--service-data-dir", str(args.service_data_dir.resolve() if args.service_data_dir else lab / "server-data")]
                        if args.rust_stun:
                            tls_argv += ["--stun-bind", "127.0.0.1:3074", "--stun-alt", "127.0.0.1:3078"]
                        if args.rust_game:
                            tls_argv += ["--game-bind", "127.0.0.1:39004"]
                            if args.quiet_network:
                                tls_argv += ["--game-capture-every", "30"]
                        if args.game_state:
                            tls_argv += ["--game-state", str(args.game_state.resolve())]
                            state["game_state_sha256"] = sha256(args.game_state)
                        if args.game_map_catalog:
                            tls_argv += ["--game-map-catalog", str(args.game_map_catalog.resolve())]
                        if args.hit_regions:
                            tls_argv += ["--hit-regions", str(args.hit_regions.resolve())]
                        if args.snapshot_probe:
                            tls_argv += ["--snapshot-probe", str(args.snapshot_probe)]
                    else:
                        openssl = "/opt/homebrew/bin/openssl" if Path("/opt/homebrew/bin/openssl").exists() else shutil.which("openssl")
                        if openssl is None:
                            raise RuntimeError("OpenSSL is required for tls-probe")
                        tls_argv = [openssl, "s_server", "-accept", "127.0.0.1:39002", "-cert", str(certificate), "-key", str(key), "-tls1", "-cipher", "AES128-SHA:@SECLEVEL=0", "-quiet", "-ign_eof", "-state"]
                    state["tls_argv"] = tls_argv
                    tls = subprocess.Popen(tls_argv, stdin=subprocess.PIPE, stdout=tls_plaintext, stderr=tls_status)
                    if args.trust_lab_cert:
                        state["cacert_before_sha256"] = sha256(work / "cacert.pem")
                        trust_lab_certificate(work, certificate, output, ca_backups)
                        state["temporary_ca_paths"] = [str(path.relative_to(work)) for path in ca_backups]
                cfg += '\n[[listeners]]\nname = "resources"\nbind = "127.0.0.1:39003"\n'
                (output / "observe.toml").write_text(cfg)
                server = subprocess.Popen([str(REPO / "target/debug/codol-server"), "observe", "--config", str(output / "observe.toml")], stdout=server_log, stderr=subprocess.STDOUT)
                deadline = time.monotonic() + 5
                while not list((output / "captures").glob("run-*/run.json")) or (args.rust_auth and not list((output / "auth-captures").glob("auth-*/run.json"))):
                    if server.poll() is not None:
                        raise RuntimeError("observer exited; see observer.log")
                    if tls is not None and tls.poll() is not None:
                        raise RuntimeError("AUTH backend exited; see tls-status.log")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("observer startup")
                    time.sleep(0.1)
                argv = [wine, str(work / "codoMP_client_shipRetail.exe"), "-q", "0", "-src=tgp", "-game_id", "29", "-zone_id", "83886082", "+set", "r_fullscreen", "0"]
            elif args.action == "init":
                argv = [wine, "wineboot.exe", "-u"]
            else:
                argv = [wine, r"C:\windows\syswow64\cmd.exe", "/c", "ver"]
            process = subprocess.Popen(argv, cwd=work if args.action == "client" else REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
            if args.action == "client" and args.crossproxy:
                cross = subprocess.Popen([wine, "CrossProxy.exe", "41c221af", "0", "0", "0", "5000002"], cwd=work / "Cross", env=env, stdout=cross_log, stderr=subprocess.STDOUT)
            state.update(pid=process.pid, argv=argv)
            (output / "experiment.json").write_text(json.dumps(state, indent=2) + "\n")
            print(json.dumps({"output": str(output), "pid": process.pid}), flush=True)
            deadline = time.monotonic() + args.seconds
            while process.poll() is None and time.monotonic() < deadline:
                if server is not None and server.poll() is not None:
                    raise RuntimeError("observer stopped during the experiment")
                if tls is not None and tls.poll() is not None:
                    raise RuntimeError("TLS probe stopped; inspect tls-status.log")
                if (output / "wine.log").stat().st_size + (output / "crossproxy.log").stat().st_size >= 64 * 1024 * 1024:
                    state["log_limit_reached"] = True
                    break
                time.sleep(0.25)
            state.update(exit_code=process.poll(),stopped_by_harness=process.poll() is None)
        except BaseException as error:
            state["harness_error"] = type(error).__name__ + ": " + str(error)
            raise
        finally:
            # A second interrupt must not strand the cleanup halfway through.
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            # WINEPREFIX scopes wineserver -k to this project-owned environment.
            try:
                stop_prefix(engine, env)
            finally:
                for child in [process, cross]:
                    if child is not None:
                        try:
                            child.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            child.kill()
                            child.wait()
                if server is not None:
                    if server.poll() is None:
                        server.send_signal(signal.SIGINT)
                    try:
                        server.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()
                if tls is not None:
                    if tls.poll() is None:
                        tls.send_signal(signal.SIGINT) if args.rust_auth else tls.terminate()
                    try:
                        tls.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        tls.kill()
                        tls.wait()
                    if tls.stdin is not None:
                        tls.stdin.close()
                if ca_backups:
                    restore_certificates(ca_backups)
                    state["lab_certificate_restored"] = True
                state["finished_unix"] = time.time()
                (output / "experiment.json").write_text(json.dumps(state, indent=2) + "\n")
                lock.close()
    print(json.dumps(state, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
