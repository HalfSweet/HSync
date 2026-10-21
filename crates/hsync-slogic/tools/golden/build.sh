#!/bin/sh
# Execute the upstream C control path; no libusb or hardware is involved.
set -eu
: "${SLOGIC:?set SLOGIC to a sipeed/libsigrok checkout on slogic-dev}"
here=$(cd "$(dirname "$0")" && pwd)
source_dir="$SLOGIC/src/hardware/sipeed-slogic-analyzer/slogic"
out=${TMPDIR:-/tmp}/hsync-slogic-golden
mkdir -p "$out"
cc -std=c11 -Wall -Wextra -Werror -O1 -I"$source_dir" \
  "$here/harness.c" "$source_dir/slogic.c" "$source_dir/slogic8u2.c" \
  "$source_dir/slogic16u3.c" "$source_dir/slogic32u3.c" -o "$out/golden"
"$out/golden" > "$here/../../tests/fixtures/protocol.txt"
echo "wrote tests/fixtures/protocol.txt"
