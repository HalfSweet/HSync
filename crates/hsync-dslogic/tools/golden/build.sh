#!/bin/sh
# Builds the golden generator against a DSView checkout and regenerates
# tests/fixtures/settings.txt from cases.txt. Usage: DSVIEW=/path/to/DSView ./build.sh
set -eu
: "${DSVIEW:?set DSVIEW to a DSView checkout}"
here=$(cd "$(dirname "$0")" && pwd)
out=${TMPDIR:-/tmp}/hsync-dslogic-golden
mkdir -p "$out"
lib="$DSVIEW/libsigrok4DSL"
cc -std=gnu11 -O1 -w -o "$out/golden" \
  -I"$lib" -I"$DSVIEW/common" \
  -I"$(pkg-config --variable=includedir libusb-1.0)" $(pkg-config --cflags glib-2.0 libusb-1.0) \
  "$here/harness.c" "$lib/hardware/DSL/dsl.c" "$lib/hardware/DSL/command.c" "$lib/trigger.c" \
  $(pkg-config --libs glib-2.0 libusb-1.0) -lm -Wl,-undefined,dynamic_lookup
"$out/golden" < "$here/cases.txt" > "$here/../../tests/fixtures/settings.txt"
echo "wrote tests/fixtures/settings.txt"
