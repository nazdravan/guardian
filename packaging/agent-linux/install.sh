#!/bin/sh
# Installs or upgrades the Guardian 3.0 agent as the systemd service
# `guardian-agent` and enrolls this computer on the first run.
#
#   sudo ./install.sh wss://server:8443 ENROLL-CODE [CERT_SHA256]
#
# Running it again upgrades the binaries and never re-enrolls.
#
# Local fallback mode: the agent listens on TCP and UDP port 11500 (set
# `local_port = N` in agent.toml, `0` = off). Most Linux desktops have no
# firewall; with ufw or firewalld, allow 11500/tcp and 11500/udp from the
# school network only.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
DIR=/opt/guardian-agent
CONFIG=/etc/guardian-agent/agent.toml
DATA=/var/lib/guardian-agent
UNIT=/etc/systemd/system/guardian-agent.service

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo $0 wss://server:port CODE [CERT_SHA256]" >&2
    exit 1
fi

install -d -m 0755 "$DIR"
install -m 0755 "$HERE/guardian-agent" "$DIR/guardian-agent"
install -m 0755 "$HERE/guardian-worker" "$DIR/guardian-worker"
install -d -m 0700 /etc/guardian-agent "$DATA"

if grep -q '^computer_id' "$CONFIG" 2>/dev/null; then
    echo "Already enrolled ($(grep '^computer_id' "$CONFIG")); keeping the enrollment."
else
    if [ $# -lt 2 ]; then
        echo "Usage: sudo $0 wss://server:port CODE [CERT_SHA256]" >&2
        exit 1
    fi
    set -- --server "$1" --code "$2" ${3:+--cert-sha256 "$3"}
    "$DIR/guardian-agent" --config "$CONFIG" --data-dir "$DATA" enroll "$@"
fi

install -m 0644 "$HERE/guardian-agent.service" "$UNIT"
systemctl daemon-reload
systemctl enable guardian-agent >/dev/null
systemctl restart guardian-agent

echo
echo "Guardian agent is running:"
"$DIR/guardian-agent" --config "$CONFIG" --data-dir "$DATA" status || true
