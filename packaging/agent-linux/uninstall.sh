#!/bin/sh
# Removes the Guardian 3.0 agent service and binaries. The enrollment
# (/etc/guardian-agent) and the saved state (/var/lib/guardian-agent) are
# kept unless --purge is given.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo $0 [--purge]" >&2
    exit 1
fi

systemctl disable --now guardian-agent >/dev/null 2>&1 || true
rm -f /etc/systemd/system/guardian-agent.service
systemctl daemon-reload
rm -rf /opt/guardian-agent

if [ "${1:-}" = "--purge" ]; then
    rm -rf /etc/guardian-agent /var/lib/guardian-agent
    echo "Guardian agent removed, with its enrollment and state."
else
    echo "Guardian agent removed. Enrollment and state kept in /etc/guardian-agent and /var/lib/guardian-agent (use --purge to delete)."
fi
