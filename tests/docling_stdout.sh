#!/bin/sh
set -e
TMPDIR=$(mktemp -d)
docling --to text "$1" --output "$TMPDIR" >/dev/null
cat "$TMPDIR"/*.txt
rm -rf "$TMPDIR"
