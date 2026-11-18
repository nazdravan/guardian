#!/usr/bin/env bash
# Checks the documentation: every relative .md link in specs/, README.md and
# AGENTS.md points at a file that exists, and every i18n key of the landing
# page has a value in all five languages.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
python3 -I - <<'PY'
import pathlib, re, sys

root = pathlib.Path(".")
errors = []
files = [root / "README.md", root / "AGENTS.md"] + sorted((root / "specs").glob("*.md"))
link = re.compile(r"\]\(([^)#\s]+\.md)(#[^)]*)?\)")
for f in files:
    text = f.read_text(encoding="utf-8")
    # Inside fenced code blocks links are examples.
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    for m in link.finditer(text):
        target = m.group(1)
        if "://" in target:
            continue
        if not (f.parent / target).resolve().exists():
            errors.append(f"{f}: broken link to {target}")

html = (root / "docs" / "index.html").read_text(encoding="utf-8")
used = set(re.findall(r'data-i18n(?:-[a-z]+)?="([a-zA-Z0-9_]+)"', html))
start = html.index("const I18N = {")
body = html[start:]
langs = {}
for lang in ("en", "ro", "ru", "es", "de"):
    m = re.search(r"\n      " + lang + r": \{(.*?)\n      \},", body, flags=re.S)
    if not m:
        errors.append(f"docs/index.html: no {lang} block")
        continue
    langs[lang] = set(re.findall(r"([a-zA-Z0-9_]+): \"", m.group(1)))
for lang, keys in langs.items():
    for k in sorted(used - keys):
        errors.append(f"docs/index.html: {lang} has no {k}")
for lang, keys in langs.items():
    extra = keys - langs.get("en", keys)
    for k in sorted(extra):
        errors.append(f"docs/index.html: {lang} has {k}, en does not")

for e in errors:
    print("FAIL:", e)
if errors:
    sys.exit(1)
print("docs: ok")
PY
