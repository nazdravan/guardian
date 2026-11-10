#!/usr/bin/env bash
# Builds every component and packs the release archive:
#
#   dist/guardian-<version>.zip
#     guardian-<version>/
#       server/          guardian-server (linux/amd64), server.env, install.sh, systemd unit, Docker files
#       agent-windows/   Guardian.exe, Install/Uninstall .bat, agent.env, bin/ (ps1 + 32/64-bit agent)
#       agent-linux/     procsentinel-agent (linux/amd64), agent.env, install.sh, uninstall.sh, systemd unit
#       guardian.apk
#
# The version comes from ./VERSION. Templates live in packaging/; dist/ is build
# output only and is not tracked by git.
#
# Needs: go, zip, and flutter (unless SKIP_APK=1).
# APK signing: set ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS and
# ANDROID_KEY_PASSWORD; without them the APK is signed with the debug key (local builds only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
BUILD_NUMBER="${BUILD_NUMBER:-1}"
NAME="guardian-$VERSION"
OUT="$ROOT/dist/$NAME"

rm -rf "$OUT" "$ROOT/dist/$NAME.zip"
mkdir -p "$OUT"
cp -R "$ROOT/packaging/server" "$ROOT/packaging/agent-windows" "$ROOT/packaging/agent-linux" "$OUT/"

gobuild() { # <module dir> <GOOS> <GOARCH> <output> [extra ldflags]
  (cd "$ROOT/$1" && CGO_ENABLED=0 GOOS="$2" GOARCH="$3" \
    go build -trimpath -ldflags "-s -w ${5:-}" -o "$4" .)
}

echo "==> server (linux/amd64)"
gobuild server linux amd64 "$OUT/server/guardian-server"

echo "==> agent (windows/amd64, windows/386, linux/amd64)"
mkdir -p "$OUT/agent-windows/bin/agent"
gobuild agent windows amd64 "$OUT/agent-windows/bin/agent/procsentinel-agent64.exe"
gobuild agent windows 386 "$OUT/agent-windows/bin/agent/procsentinel-agent32.exe"
gobuild agent linux amd64 "$OUT/agent-linux/procsentinel-agent"

echo "==> Guardian Console (windows/386)"
# Same steps as tools/whitelist-gui/build.ps1: embed manifest + icon, GUI subsystem.
GUI="$ROOT/tools/whitelist-gui"
SYSO="$GUI/rsrc_windows_386.syso"
trap 'rm -f "$SYSO"' EXIT
(cd "$GUI" && go run github.com/akavel/rsrc@v0.10.2 -manifest whitelist-gui.manifest -ico guardian.ico -arch 386 -o "$SYSO")
gobuild tools/whitelist-gui windows 386 "$OUT/agent-windows/Guardian.exe" "-H windowsgui"
rm -f "$SYSO"

if [ "${SKIP_APK:-0}" != "1" ]; then
  echo "==> mobile (apk)"
  (cd "$ROOT/mobile" && flutter pub get && \
    flutter build apk --release --build-name="$VERSION" --build-number="$BUILD_NUMBER")
  cp "$ROOT/mobile/build/app/outputs/flutter-apk/app-release.apk" "$OUT/guardian.apk"
else
  echo "==> mobile skipped (SKIP_APK=1)"
fi

echo "==> $NAME.zip"
(cd "$ROOT/dist" && zip -qr "$NAME.zip" "$NAME")
echo "dist/$NAME.zip"
