#!/usr/bin/env python3
"""Start the local 3.3.3.3 game with the prepared native Bot map catalog."""
import argparse
from pathlib import Path
import subprocess
import sys

from clientlib import REPO


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix-name",default="wine10-minimap-20261010")
    parser.add_argument("--map-catalog",type=Path,default=REPO/"research-output/native-three-map-catalog.json")
    parser.add_argument("--game-state",type=Path,default=REPO/"research-output/tdm-afghan-hud-state.json")
    parser.add_argument("--seconds",type=int,default=0,help="optional bounded verification run; default: until exit")
    args=parser.parse_args()
    if not args.map_catalog.is_file() or not args.game_state.is_file():
        parser.error("prepared map catalog/state missing; see docs/LOCAL_PLAY.md")
    if not 0<=args.seconds<=3600:parser.error("seconds must be 0..3600")
    subprocess.run(["sh",str(REPO/"tools/cargo.sh"),"build","--release","--locked","-p","codol-server"],cwd=REPO,check=True)
    command=[sys.executable,str(REPO/"tools/gamehub_run.py"),"client",
             "--prefix-name",args.prefix_name,"--seconds",str(args.seconds),"--continuous-play",
             "--title-id-hex","3ea","--rust-auth","--release-server","--local-auth-tickets",
             "--rust-lobby","--rust-stun","--rust-game","--local-services","--trust-lab-cert","--quiet-network",
             "--native-bot-rooms","--game-state",str(args.game_state.resolve()),"--game-map-catalog",str(args.map_catalog.resolve())]
    # Replace the wrapper so SIGINT/SIGTERM reach the harness and its cleanup.
    import os
    os.chdir(REPO)
    os.execv(sys.executable,command)


if __name__=="__main__":
    main()
