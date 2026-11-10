#!/bin/bash
# Removes the Guardian agent service. Pass --purge to also delete /opt/guardian-agent
# (its .env, whitelist.txt and sync.json).
set -e

if [ "$EUID" -ne 0 ]; then
  exec sudo bash "$0" "$@"
fi

systemctl disable --now guardian-agent 2>/dev/null || true
rm -f /etc/systemd/system/guardian-agent.service
systemctl daemon-reload
if [ "$1" = "--purge" ]; then
  rm -rf /opt/guardian-agent
  echo "Removed /opt/guardian-agent"
else
  echo "Service removed; configuration kept in /opt/guardian-agent"
fi
