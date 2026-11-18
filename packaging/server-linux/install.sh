#!/bin/sh
# Installs or upgrades the Guardian 3.0 server as the systemd service
# `guardian-server`. Safe to run again: the data directory (database,
# certificate) is kept, so the certificate pin does not change.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
BIN=/usr/local/bin/guardian-server
UNIT=/etc/systemd/system/guardian-server.service

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo $0" >&2
    exit 1
fi

if ! id guardian >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir /var/lib/guardian \
        --shell /usr/sbin/nologin guardian
fi

install -m 0755 "$HERE/guardian-server" "$BIN"
install -m 0755 "$HERE/guardian-cli" /usr/local/bin/guardian-cli
install -m 0644 "$HERE/guardian-server.service" "$UNIT"

systemctl daemon-reload
systemctl enable guardian-server >/dev/null
systemctl restart guardian-server

# Wait for the startup lines, then show what the admin needs.
i=0
while [ "$i" -lt 30 ]; do
    if journalctl -u guardian-server -b --no-pager -q | grep -q "certificate sha256:"; then
        break
    fi
    i=$((i + 1))
    sleep 1
done

echo
echo "Guardian server is running."
journalctl -u guardian-server -b --no-pager -q -o cat \
    | grep -E "listening on|certificate sha256:|bootstrap code:" | tail -n 3 || true
echo
echo "Give the certificate sha256 to the console and the agents (pinning)."
echo "The bootstrap code is shown only until the first admin is created."
