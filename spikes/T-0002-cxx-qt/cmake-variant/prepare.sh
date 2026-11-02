#!/bin/sh
# Copies the shared sources from the Cargo-driven spike into the CMake variant (not committed).
set -e
cd "$(dirname "$0")"
for f in bench counters image metrics model; do cp ../src/$f.rs rust/src/$f.rs; done
rm -rf rust/qml rust/cpp; cp -r ../qml rust/qml; cp -r ../cpp rust/cpp
