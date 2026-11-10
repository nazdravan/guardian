#!/bin/bash
# Installs or upgrades the Guardian agent on Linux as a systemd service.
# Run it from the unpacked agent-linux/ folder of the release archive.
set -e

if [ "$EUID" -ne 0 ]; then
  echo "Switching to root..."
  exec sudo bash "$0" "$@"
fi

cd "$(dirname "$0")"
DIR=/opt/guardian-agent

mkdir -p "$DIR"
systemctl stop guardian-agent 2>/dev/null || true
install -m 0755 procsentinel-agent "$DIR/procsentinel-agent"
# An upgrade keeps the configuration: .env is only written on first install.
if [ ! -f "$DIR/.env" ]; then
  install -m 0600 agent.env "$DIR/.env"
  echo "Created $DIR/.env from agent.env - set SERVER_ADDRESS and TOKEN there."
else
  echo "Keeping existing $DIR/.env"
fi
install -m 0644 guardian-agent.service /etc/systemd/system/guardian-agent.service
systemctl daemon-reload
systemctl enable guardian-agent
systemctl restart guardian-agent
systemctl status guardian-agent --no-pager
