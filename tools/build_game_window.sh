#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .tools
clang -Wno-deprecated-declarations tools/game_window_focus.c -framework Carbon -o .tools/game-window-focus
swiftc tools/game_window.swift -o .tools/game-window
