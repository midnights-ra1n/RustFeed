#!/bin/sh
# Installs the latest RustFeed .deb from GitHub. Run as root by rustfeed-update.service
# when the web interface drops the trigger file; the trigger carries no data, so the
# unprivileged app cannot choose what gets installed.
set -eu

rm -f /var/lib/rustfeed/data/update-requested

arch=$(dpkg --print-architecture)
url=$(curl -fsSL https://api.github.com/repos/midnights-ra1n/RustFeed/releases/latest \
  | grep -o "https://github.com/midnights-ra1n/RustFeed/releases/download/[^\"]*_${arch}\.deb" | head -n 1)
[ -n "$url" ] || { echo "No .deb for $arch in the latest release" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/rustfeed.deb" "$url"
dpkg -i "$tmp/rustfeed.deb"
