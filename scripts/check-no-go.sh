#!/usr/bin/env bash
# Fails while Go code from Guardian 2.x is still in the repo. With --docs it
# also checks that the main documents no longer point at the deleted paths.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
fail=0

left="$(git ls-files -- '*.go' 'go.mod' 'go.sum' '**/go.mod' '**/go.sum' ':!:mobile/**')"
if [ -n "$left" ]; then
  echo "FAIL: Go files still tracked:"; echo "$left" | head -20; fail=1
fi
for d in server agent tools/server-gui tools/whitelist-gui tools/mkico; do
  if [ -n "$(git ls-files -- "$d")" ]; then echo "FAIL: $d/ is still tracked"; fail=1; fi
done

if [ "${1:-}" = "--docs" ]; then
  for f in AGENTS.md README.md .github; do
    hits="$(git grep -nE 'tools/whitelist-gui|tools/server-gui|server/guardian|agent/main\.go|procsentinel-agent' -- "$f" || true)"
    if [ -n "$hits" ]; then echo "FAIL: $f mentions deleted paths:"; echo "$hits" | head -10; fail=1; fi
  done
fi

[ "$fail" -eq 0 ] && echo "no Go left: ok"
exit "$fail"
