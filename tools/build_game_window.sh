#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
codol_window_output="${1:-.tools}"
mkdir -p "$codol_window_output"
clang -Wno-deprecated-declarations tools/game_window_focus.c -framework Carbon -o "$codol_window_output/game-window-focus"
swiftc tools/game_window.swift -o "$codol_window_output/game-window"
