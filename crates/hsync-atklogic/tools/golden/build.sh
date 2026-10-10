#!/bin/sh
# Builds the golden generator against an ATK-Logic checkout and regenerates
# tests/fixtures/protocol.txt from cases.txt. Usage: ATK=/path/to/atk-logic ./build.sh
set -eu
: "${ATK:?set ATK to an ATK-Logic checkout}"
here=$(cd "$(dirname "$0")" && pwd)
out=${TMPDIR:-/tmp}/hsync-atklogic-golden
mkdir -p "$out"
# The CRC table and function, and the two lane conversions, as ATK-Logic has them.
{
  sed -n '/^const quint32 table\[\] = {/,/^};/p' "$ATK/pv/static/util.cpp"
  sed -n '/^quint32 gCRC32/,/^}/p' "$ATK/pv/static/util.cpp"
  sed -n '/^void logic_analyzer_convert_to_pc/,/^}/p' "$ATK/pv/usb/usb_base.cpp"
  sed -n '/^void logic_analyzer_convert_to_device/,/^}/p' "$ATK/pv/usb/usb_base.cpp"
} > "$out/extracted.inc"
c++ -std=c++17 -O1 -w -I"$out" -o "$out/golden" "$here/harness.cpp"
"$out/golden" < "$here/cases.txt" > "$here/../../tests/fixtures/protocol.txt"
echo "wrote tests/fixtures/protocol.txt"
