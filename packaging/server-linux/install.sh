#!/bin/bash
# Installs or upgrades the Guardian server as a systemd service.
# Run it from the unpacked server/ folder of the release archive.
set -e

if [ "$EUID" -ne 0 ]; then
  echo "Switching to root..."
  exec sudo bash "$0" "$@"
fi

cd "$(dirname "$0")"
DIR=/usr/local/bin/guardian

mkdir -p "$DIR"
systemctl stop guardian-server 2>/dev/null || true
install -m 0755 guardian-server "$DIR/guardian-server"
# An upgrade keeps the configured tokens: .env is only written on first install.
if [ ! -f "$DIR/.env" ]; then
  install -m 0600 server.env "$DIR/.env"
  echo "Created $DIR/.env from server.env - set TOKEN and ADMIN_TOKEN there."
else
  echo "Keeping existing $DIR/.env"
fi
install -m 0644 guardian-server.service /etc/systemd/system/guardian-server.service
systemctl daemon-reload
systemctl enable guardian-server
systemctl restart guardian-server
systemctl status guardian-server --no-pager
