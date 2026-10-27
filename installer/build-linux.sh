#!/bin/sh
# Builds Charpick for Linux and packs the release download:
# dist/charpick-linux-<arch>.tar.gz, a folder "charpick" with the program, install.sh
# (this folder's install-linux.sh), the README and the license. Unpacked,
# `sh charpick/install.sh` installs it without building anything.
#
#   sh installer/build-linux.sh

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

cargo build --release --locked

archive=dist/charpick-linux-$(uname -m).tar.gz
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/charpick"
install -m 755 target/release/Charpick "$stage/charpick/charpick"
install -m 755 installer/install-linux.sh "$stage/charpick/install.sh"
install -m 644 README.md LICENSE "$stage/charpick/"

mkdir -p dist
tar -czf "$archive" -C "$stage" --owner=0 --group=0 charpick
echo "Built $root/$archive"
