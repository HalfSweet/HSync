#!/bin/sh
# Builds the golden generator against PXLogic's libsigrok (branch pxview-fork, as PXView pins
# it) and regenerates tests/fixtures/sequences.txt from cases.txt.
# Usage: PXLIB=/path/to/PXLogic/libsigrok ./build.sh
set -eu
: "${PXLIB:?set PXLIB to a checkout of the PXLogic libsigrok}"
here=$(cd "$(dirname "$0")" && pwd)
out=${TMPDIR:-/tmp}/hsync-pxlogic-golden
mkdir -p "$out/libsigrok"
sed -e 's/@SR_PACKAGE_VERSION_MAJOR@/0/; s/@SR_PACKAGE_VERSION_MINOR@/6/' \
    -e 's/@SR_PACKAGE_VERSION_MICRO@/0/; s/@SR_PACKAGE_VERSION@/0.6.0/' \
    -e 's/@SR_LIB_VERSION_CURRENT@/0/; s/@SR_LIB_VERSION_REVISION@/0/' \
    -e 's/@SR_LIB_VERSION_AGE@/0/; s/@SR_LIB_VERSION@/0:0:0/' \
    "$PXLIB/include/libsigrok/version.h.in" > "$out/libsigrok/version.h"
cc -std=gnu11 -O1 -w -o "$out/golden" \
  -I"$out" -I"$PXLIB/include" -I"$PXLIB/src" -I"$PXLIB/src/hardware/pxlogic" \
  $(pkg-config --cflags glib-2.0 libusb-1.0) \
  "$here/harness.c" "$here/unreached.c" "$PXLIB/src/hardware/pxlogic/usb_ctrl.c" \
  $(pkg-config --libs glib-2.0)
"$out/golden" < "$here/cases.txt" > "$here/../../tests/fixtures/sequences.txt"
echo "wrote tests/fixtures/sequences.txt"
