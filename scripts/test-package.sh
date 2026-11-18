#!/usr/bin/env bash
# Tests scripts/package.sh with fake binaries: the archive's layout, nothing
# from 2.x in it, and a clear failure when a binary is missing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

fake() { # <dir> <names...>
  local d="$1"; shift
  mkdir -p "$d"
  for n in "$@"; do printf 'fake %s/%s\n' "$(basename "$d")" "$n" > "$d/$n"; done
}
fake "$T/linux" guardian-server guardian-cli guardian-agent guardian-worker
fake "$T/win64" guardian-server.exe guardian-cli.exe guardian-agent.exe guardian-worker.exe
fake "$T/win32" guardian-agent.exe guardian-worker.exe guardian-console.exe guardian-setup.exe

"$ROOT/scripts/package.sh" --linux "$T/linux" --win64 "$T/win64" --win32 "$T/win32" --out "$T/out" > "$T/log" 2>&1 \
  || { cat "$T/log"; echo "FAIL: package.sh failed"; exit 1; }

ZIP="$T/out/guardian-$VERSION.zip"
[ -f "$ZIP" ] || { echo "FAIL: $ZIP missing"; exit 1; }
LIST="$(unzip -Z1 "$ZIP")"
N="guardian-$VERSION"
for p in \
  server-linux/guardian-server server-linux/guardian-cli server-linux/guardian-server.service server-linux/install.sh \
  server-windows/guardian-server.exe server-windows/guardian-cli.exe server-windows/README.txt \
  console-windows/guardian-console.exe console-windows/README.txt \
  agent-windows/guardian-setup.exe agent-windows/README.txt \
  agent-windows/bin/x64/guardian-agent.exe agent-windows/bin/x64/guardian-worker.exe \
  agent-windows/bin/x86/guardian-agent.exe agent-windows/bin/x86/guardian-worker.exe \
  agent-linux/guardian-agent agent-linux/guardian-worker agent-linux/guardian-agent.service \
  agent-linux/install.sh agent-linux/uninstall.sh
do
  grep -qx "$N/$p" <<<"$LIST" || { echo "FAIL: $N/$p is not in the archive"; echo "$LIST"; exit 1; }
done
if grep -Eq '\.apk$|\.ps1$|\.bat$|\.env$|procsentinel' <<<"$LIST"; then
  echo "FAIL: 2.x files in the archive:"; grep -E '\.apk$|\.ps1$|\.bat$|\.env$|procsentinel' <<<"$LIST"; exit 1
fi
# The 64-bit and 32-bit agents are the right ones.
X64="$(unzip -p "$ZIP" "$N/agent-windows/bin/x64/guardian-agent.exe")"
[ "$X64" = "fake win64/guardian-agent.exe" ] || { echo "FAIL: x64 agent: $X64"; exit 1; }
X86="$(unzip -p "$ZIP" "$N/agent-windows/bin/x86/guardian-agent.exe")"
[ "$X86" = "fake win32/guardian-agent.exe" ] || { echo "FAIL: x86 agent: $X86"; exit 1; }

# A missing binary stops the script with its name.
rm "$T/win32/guardian-setup.exe"
if "$ROOT/scripts/package.sh" --linux "$T/linux" --win64 "$T/win64" --win32 "$T/win32" --out "$T/out2" > "$T/log2" 2>&1; then
  echo "FAIL: package.sh succeeded without guardian-setup.exe"; exit 1
fi
grep -q "guardian-setup.exe" "$T/log2" || { cat "$T/log2"; echo "FAIL: the error does not name the binary"; exit 1; }
echo "package.sh: ok"
