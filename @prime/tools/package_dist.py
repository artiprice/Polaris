"""Package the Polaris submission build (P10-4).

Expects the binaries already built with the `dist` profile and the web UI's site built by
cargo-leptos:

    cargo build --profile dist -p polaris-cli --bin polaris -p polaris-daemon --bin polarisd \
        -p polaris-desktop --bin polaris-desktop -p polaris-web --bin polaris-web \
        -p polaris-worker --bin polaris-worker
    (`--bin` filters apply to every package of one invocation, so each binary is named.)
    cd apps/polaris-web && cargo leptos build --release --frontend-only

Copies them with README and LICENSE into `target/package/polaris-<version>-<target>/`, zips the
folder, and writes `SHA256SUMS.txt` and `build-info.json` both into the package and into
`@prime/evidence/p10/` (the binaries themselves are not committed).

Run from the repository root:  python -I @prime/tools/package_dist.py
"""

import hashlib
import json
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BINARIES = ["polaris", "polarisd", "polaris-desktop", "polaris-web", "polaris-worker"]


def run(*args: str) -> str:
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    rustc = run("rustc", "-vV")
    host = next(line.split(": ", 1)[1] for line in rustc.splitlines() if line.startswith("host: "))
    version = next(
        line.split("=", 1)[1].strip().strip('"')
        for line in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines()
        if line.startswith("version =")
    )
    exe = ".exe" if host.endswith("windows-msvc") or host.endswith("windows-gnu") else ""
    built = ROOT / "target" / "dist"
    site = ROOT / "target" / "site"
    missing = [name for name in BINARIES if not (built / f"{name}{exe}").is_file()]
    if missing or not (site / "pkg").is_dir():
        print(f"missing build outputs: {missing or ''} {'' if (site / 'pkg').is_dir() else 'target/site/pkg'}")
        return 1

    name = f"polaris-{version}-{host}"
    package = ROOT / "target" / "package" / name
    if package.exists():
        shutil.rmtree(package)
    (package / "bin").mkdir(parents=True)
    files = []
    for binary in BINARIES:
        source = built / f"{binary}{exe}"
        target = package / "bin" / source.name
        shutil.copy2(source, target)
        files.append(target)
    shutil.copytree(site, package / "site")
    for document in ["README.md", "LICENSE"]:
        shutil.copy2(ROOT / document, package / document)

    entries = sorted(
        (path for path in package.rglob("*") if path.is_file()),
        key=lambda path: path.relative_to(package).as_posix(),
    )
    sums = "".join(f"{sha256(path)}  {path.relative_to(package).as_posix()}\n" for path in entries)
    info = {
        "package": name,
        "version": version,
        "target": host,
        "profile": "dist (release + fat LTO, codegen-units 1, overflow checks, panic=unwind, stripped)",
        "rustc": next(line for line in rustc.splitlines() if line.startswith("rustc ")),
        "commit": run("git", "rev-parse", "HEAD"),
        "dirty": bool(run("git", "status", "--porcelain", "--untracked-files=no")),
        "binaries": [
            {"name": path.name, "bytes": path.stat().st_size, "sha256": sha256(path)} for path in files
        ],
        "site_files": sum(1 for path in (package / "site").rglob("*") if path.is_file()),
        "run": "bin/polaris (terminal), bin/polaris-desktop, bin/polaris-web --site-root site, bin/polarisd",
    }
    (package / "SHA256SUMS.txt").write_bytes(sums.encode())
    (package / "build-info.json").write_bytes((json.dumps(info, indent=2) + "\n").encode())

    archive = package.parent / f"{name}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as handle:
        for path in sorted(package.rglob("*")):
            if path.is_file():
                handle.write(path, f"{name}/{path.relative_to(package).as_posix()}")
    info["archive"] = {"name": archive.name, "bytes": archive.stat().st_size, "sha256": sha256(archive)}

    evidence = ROOT / "@prime" / "evidence" / "p10"
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / "SHA256SUMS.txt").write_bytes(sums.encode())
    (evidence / "build-info.json").write_bytes((json.dumps(info, indent=2) + "\n").encode())
    print(json.dumps(info, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
