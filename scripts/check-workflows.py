"""Checks the GitHub workflows: they load, the release has its three jobs in
order, and nothing still builds 2.x (Go, Flutter, the Go tools).

    python3 -I scripts/check-workflows.py
"""

import pathlib
import re
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - CI has PyYAML
    print("PyYAML is needed: pip install pyyaml")
    sys.exit(2)

ROOT = pathlib.Path(__file__).resolve().parent.parent
WF = ROOT / ".github" / "workflows"
FORBIDDEN = [r"setup-go", r"go-version", r"(?<![A-Za-z-])go (vet|test|build|run)\b",
             r"flutter", r"whitelist-gui", r"server-gui", r"-v3/", r"ANDROID_KEYSTORE"]

errors = []
loaded = {}
for f in sorted(WF.glob("*.yml")):
    text = f.read_text(encoding="utf-8")
    try:
        loaded[f.name] = yaml.safe_load(text)
    except yaml.YAMLError as e:
        errors.append(f"{f.name}: {e}")
        continue
    for word in FORBIDDEN:
        if re.search(word, text):
            errors.append(f"{f.name}: still mentions {word!r}")

rel = loaded.get("release.yml") or {}
jobs = rel.get("jobs") or {}
for j in ("linux", "windows", "package"):
    if j not in jobs:
        errors.append(f"release.yml: no job {j!r}")
needs = (jobs.get("package") or {}).get("needs") or []
if isinstance(needs, str):
    needs = [needs]
for j in ("linux", "windows"):
    if j not in needs:
        errors.append(f"release.yml: package does not need {j!r}")
on = rel.get("on", rel.get(True)) or {}
branches = ((on.get("push") or {}).get("branches")) if isinstance(on, dict) else None
if branches != ["main"]:
    errors.append(f"release.yml: must run on push to main only, has {branches!r}")
steps = " ".join(str(s) for s in (jobs.get("package") or {}).get("steps") or [])
if "scripts/package.sh" not in steps:
    errors.append("release.yml: package does not run scripts/package.sh")
win = " ".join(str(s) for s in (jobs.get("windows") or {}).get("steps") or [])
for need in ("guardian-setup", "i686-pc-windows-msvc", "x86_64-pc-windows-msvc", "WINDOWS_CERT_PFX_BASE64"):
    if need not in win:
        errors.append(f"release.yml: the windows job never mentions {need!r}")
rust = " ".join(str(s) for s in ((loaded.get("rust.yml") or {}).get("jobs") or {}).get("windows", {}).get("steps") or [])
if "guardian-setup" not in rust:
    errors.append("rust.yml: the windows job does not test or build guardian-setup")

for e in errors:
    print("FAIL:", e)
if errors:
    sys.exit(1)
print("workflows: ok")
