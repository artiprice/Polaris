"""Licence audit of the resolved dependency graph against deny.toml (offline, no cargo-deny; P10-2).

Run from the repository root:  python -I @prime/tools/licence_audit.py .

Prints one line per package whose SPDX expression is not satisfied by the allow list (or the
crate's exception), and a summary by licence expression.
"""
import json
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

root = Path(sys.argv[1])
deny = tomllib.loads((root / "deny.toml").read_text(encoding="utf-8"))
allow = set(deny["licenses"]["allow"])
exceptions = {e["crate"]: set(e["allow"]) for e in deny["licenses"].get("exceptions", [])}
clarify = {c["crate"]: c.get("expression") for c in deny["licenses"].get("clarify", [])}

metadata = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
)
# Only packages reachable from the workspace on the host (all targets, default features).
reachable = {node["id"] for node in metadata["resolve"]["nodes"]}
packages = [p for p in metadata["packages"] if p["id"] in reachable]
workspace = set(metadata["workspace_members"])


def satisfied(expression: str, allowed: set[str]) -> bool:
    """Evaluate an SPDX expression (OR / AND / WITH, parentheses) against an allow set."""
    tokens = re.findall(r"\(|\)|[A-Za-z0-9.+\-]+(?:\s+WITH\s+[A-Za-z0-9.+\-]+)?", expression.replace("/", " OR "))

    def parse_or(i):
        value, i = parse_and(i)
        while i < len(tokens) and tokens[i] == "OR":
            right, i = parse_and(i + 1)
            value = value or right
        return value, i

    def parse_and(i):
        value, i = parse_atom(i)
        while i < len(tokens) and tokens[i] == "AND":
            right, i = parse_atom(i + 1)
            value = value and right
        return value, i

    def parse_atom(i):
        if tokens[i] == "(":
            value, i = parse_or(i + 1)
            return value, i + 1
        token = re.sub(r"\s+", " ", tokens[i])
        return token in allowed, i + 1

    try:
        value, end = parse_or(0)
        return value and end == len(tokens)
    except IndexError:
        return False


expressions = Counter()
problems = []
for package in packages:
    if package["id"] in workspace:
        continue
    expression = clarify.get(package["name"]) or package.get("license")
    if not expression:
        problems.append((package["name"], package["version"], "no licence field", package.get("license_file")))
        continue
    expressions[expression] += 1
    allowed = allow | exceptions.get(package["name"], set())
    if not satisfied(expression, allowed):
        problems.append((package["name"], package["version"], expression, None))

print(f"third-party packages in the resolved graph: {sum(1 for p in packages if p['id'] not in workspace)}")
print(f"workspace members: {len(workspace)}")
print("licence expressions (count):")
for expression, count in expressions.most_common():
    print(f"  {count:4}  {expression}")
print(f"packages not satisfied by deny.toml: {len(problems)}")
for name, version, expression, file in problems:
    print(f"  {name} {version}: {expression}" + (f" (licence file {Path(file).name})" if file else ""))
