#!/usr/bin/env bash
# Packs the Guardian 3.0 release archive from binaries that are already built
# (CI builds them on Linux and Windows):
#
#   guardian-<VERSION>.zip
#     guardian-<VERSION>/
#       server-linux/    guardian-server, guardian-cli, guardian-server.service, install.sh
#       server-windows/  guardian-server.exe, guardian-cli.exe (64-bit), README.txt
#       console-windows/ guardian-console.exe (32-bit), README.txt
#       agent-windows/   guardian-setup.exe (32-bit), README.txt, bin/x64/…, bin/x86/…
#       agent-linux/     guardian-agent, guardian-worker, guardian-agent.service, install.sh, uninstall.sh
#
#   scripts/package.sh --linux DIR --win64 DIR --win32 DIR [--out DIR]
#
# The version comes from ./VERSION; the templates from packaging/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LINUX="" WIN64="" WIN32="" OUTDIR="$ROOT/dist"
while [ $# -gt 0 ]; do
  case "$1" in
    --linux) LINUX="$2"; shift 2 ;;
    --win64) WIN64="$2"; shift 2 ;;
    --win32) WIN32="$2"; shift 2 ;;
    --out) OUTDIR="$2"; shift 2 ;;
    *) echo "usage: $0 --linux DIR --win64 DIR --win32 DIR [--out DIR]" >&2; exit 2 ;;
  esac
done
for d in "$LINUX" "$WIN64" "$WIN32"; do
  [ -n "$d" ] && [ -d "$d" ] || { echo "usage: $0 --linux DIR --win64 DIR --win32 DIR [--out DIR]" >&2; exit 2; }
done

VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
NAME="guardian-$VERSION"
OUT="$OUTDIR/$NAME"

need() { # <dir> <file>
  [ -f "$1/$2" ] || { echo "error: $1/$2 is missing" >&2; exit 1; }
}
for f in guardian-server guardian-cli guardian-agent guardian-worker; do need "$LINUX" "$f"; done
for f in guardian-server.exe guardian-cli.exe guardian-agent.exe guardian-worker.exe; do need "$WIN64" "$f"; done
for f in guardian-agent.exe guardian-worker.exe guardian-console.exe guardian-setup.exe; do need "$WIN32" "$f"; done

rm -rf "$OUT" "$OUTDIR/$NAME.zip"
mkdir -p "$OUT"
for d in server-linux server-windows console-windows agent-windows agent-linux; do
  cp -R "$ROOT/packaging/$d" "$OUT/"
done

install -m 0755 "$LINUX/guardian-server" "$LINUX/guardian-cli" "$OUT/server-linux/"
install -m 0755 "$LINUX/guardian-agent" "$LINUX/guardian-worker" "$OUT/agent-linux/"
cp "$WIN64/guardian-server.exe" "$WIN64/guardian-cli.exe" "$OUT/server-windows/"
cp "$WIN32/guardian-console.exe" "$OUT/console-windows/"
cp "$WIN32/guardian-setup.exe" "$OUT/agent-windows/"
mkdir -p "$OUT/agent-windows/bin/x64" "$OUT/agent-windows/bin/x86"
cp "$WIN64/guardian-agent.exe" "$WIN64/guardian-worker.exe" "$OUT/agent-windows/bin/x64/"
cp "$WIN32/guardian-agent.exe" "$WIN32/guardian-worker.exe" "$OUT/agent-windows/bin/x86/"

(cd "$OUTDIR" && zip -qr "$NAME.zip" "$NAME")
echo "$OUTDIR/$NAME.zip"
