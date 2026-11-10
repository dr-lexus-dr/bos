#!/bin/bash
# Create the standard bOS directories in the host workspace.
set -e
BOS_ROOT="${BOS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$BOS_ROOT"
mkdir -p applications system/{logs,mounts,tmp} \
    users/default/{downloads,documents,pictures,music,videos}

echo "Starting bOS-like operating system..."
echo "Root structure: system users applications"
echo "Tools:"
echo "  system/bpt           package manager (~BPT-PROGS)"
echo "  system/network       internet support"
echo "  system/text-editor   standard text editor (btext)"
echo "$(date -u +%FT%TZ) bOS init: structure ready" >> system/logs/system.log
