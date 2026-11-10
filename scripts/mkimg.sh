#!/bin/bash
set -e
# mk bFS image and put userspace binaries into /sbin /bin
IMG="${1:-target/bos.img}"
mkdir -p target
echo "mkfs.bfs $IMG (stub)"
