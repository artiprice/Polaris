#!/usr/bin/env python3
"""Assemble an allowlisted Cargo context for the locked polaris-guest workspace build."""

from __future__ import annotations

import os
import shutil
import sys
import tomllib
from pathlib import Path


ROOT_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml")
PACKAGE_INPUTS = ("src", "proto")
GUEST_BUILD_SUPPORT_FILES = {
    Path("crates/polaris-proto"): ("build_support.rs",),
}
EXPECTED_GUEST_PACKAGES = {
    "apps/polaris-guest",
    "crates/polaris-browser",
    "crates/polaris-log",
    "crates/polaris-proto",
    "crates/polaris-sdk",
}
FORBIDDEN_NAMES = {".aws", ".ssh", ".codex", ".env", ".netrc", "credentials", "credentials.toml"}


def reject_sensitive_path(path: Path) -> None:
    names = {part.lower() for part in path.parts}
    if names & FORBIDDEN_NAMES or path.name.lower().startswith(".env."):
        raise RuntimeError(f"sensitive-looking input is forbidden in Docker context: {path}")
    if path.suffix.lower() in {".pem", ".key"}:
        raise RuntimeError(f"private-key input is forbidden in Docker context: {path}")


def inside(root: Path, path: Path) -> Path:
    resolved = path.resolve(strict=True)
    try:
        return resolved.relative_to(root)
    except ValueError as error:
        raise RuntimeError(f"input escapes workspace root: {path}") from error


def copy_file(root: Path, relative: Path, destination: Path) -> None:
    reject_sensitive_path(relative)
    source = root / relative
    inside(root, source)
    if not source.is_file() or source.is_symlink():
        raise RuntimeError(f"required build input is not a regular file: {relative}")
    target = destination / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def copy_tree(root: Path, relative: Path, destination: Path) -> None:
    source = root / relative
    inside(root, source)
    if not source.is_dir() or source.is_symlink():
        raise RuntimeError(f"required build input is not a directory: {relative}")
    for current, dirs, files in os.walk(source, followlinks=False):
        current_path = Path(current)
        for name in tuple(dirs):
            if (current_path / name).is_symlink():
                raise RuntimeError(f"symlink is forbidden in build input: {current_path / name}")
        for name in files:
            path = current_path / name
            if path.is_symlink() or not path.is_file():
                raise RuntimeError(f"non-regular file is forbidden in build input: {path}")
            relative_file = path.relative_to(root)
            reject_sensitive_path(relative_file)
            target = destination / relative_file
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)


def copy_workspace_vendor_packages(
    root: Path, workspace: dict, destination: Path
) -> set[Path]:
    """Copy only vendor roots named by workspace path dependencies.

    Cargo resolves every workspace member manifest even when the Docker target is one package.
    The allowlisted context therefore needs the package source for path dependencies referenced
    by those manifests. Do not copy the entire vendor tree: derive roots from the workspace
    dependency table and verify each copied package's declared name.
    """
    dependencies = workspace.get("workspace", {}).get("dependencies", {})
    if not isinstance(dependencies, dict):
        raise RuntimeError("workspace dependencies must be a table")

    copied: set[Path] = set()
    for dependency_name, specification in sorted(dependencies.items()):
        if not isinstance(specification, dict):
            continue
        dependency_path = specification.get("path")
        if not isinstance(dependency_path, str):
            continue
        resolved = (root / dependency_path).resolve(strict=True)
        try:
            relative = resolved.relative_to(root)
        except ValueError as error:
            raise RuntimeError(
                f"workspace path dependency escapes workspace root: {dependency_name}"
            ) from error
        if not relative.parts or relative.parts[0] != "vendor":
            continue
        if relative in copied:
            continue
        manifest_path = resolved / "Cargo.toml"
        inside(root, manifest_path)
        if not resolved.is_dir() or not manifest_path.is_file() or manifest_path.is_symlink():
            raise RuntimeError(
                f"workspace vendor path dependency is not a package: {dependency_name}"
            )
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        package_name = manifest.get("package", {}).get("name")
        if package_name != dependency_name:
            raise RuntimeError(
                f"vendor package {relative} name {package_name!r} does not match workspace dependency {dependency_name!r}"
            )
        copy_tree(root, relative, destination)
        copied.add(relative)
    return copied


def copy_guest_build_support(root: Path, guest_packages: set[Path], destination: Path) -> None:
    """Copy explicitly inventoried sibling modules imported by guest build scripts."""
    for package_dir, filenames in sorted(GUEST_BUILD_SUPPORT_FILES.items()):
        if package_dir not in guest_packages:
            continue
        for filename in filenames:
            copy_file(root, package_dir / filename, destination)


def dependency_specs(manifest: dict) -> list[tuple[str, dict]]:
    found: list[tuple[str, dict]] = []

    def walk(value: dict) -> None:
        for key, child in value.items():
            if key == "dependencies" or key.endswith("-dependencies"):
                if isinstance(child, dict):
                    found.extend(
                        (name, spec if isinstance(spec, dict) else {})
                        for name, spec in child.items()
                    )
            elif isinstance(child, dict):
                walk(child)

    walk(manifest)
    return found


def guest_dependency_closure(root: Path, workspace: dict) -> set[Path]:
    workspace_dependencies = workspace.get("workspace", {}).get("dependencies", {})
    pending = [Path("apps/polaris-guest")]
    packages: set[Path] = set()
    while pending:
        package_dir = Path(os.path.normpath(pending.pop()))
        if package_dir in packages:
            continue
        manifest_path = root / package_dir / "Cargo.toml"
        inside(root, manifest_path)
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        packages.add(package_dir)
        for dependency_name, specification in dependency_specs(manifest):
            dependency_root = root / package_dir
            if specification.get("workspace") is True:
                root_specification = workspace_dependencies.get(dependency_name, {})
                specification = root_specification if isinstance(root_specification, dict) else {}
                dependency_root = root
            dependency_path = specification.get("path")
            if dependency_path:
                resolved = (dependency_root / dependency_path).resolve(strict=True)
                pending.append(resolved.relative_to(root))
    return packages


def workspace_package_dirs(root: Path, workspace: dict) -> set[Path]:
    members = workspace.get("workspace", {}).get("members", [])
    packages: set[Path] = set()
    for pattern in members:
        for candidate in root.glob(pattern):
            if candidate.is_dir() and (candidate / "Cargo.toml").is_file():
                packages.add(candidate.relative_to(root))
    return packages


def target_entrypoints(manifest: dict) -> set[str]:
    package = manifest.get("package", {})
    targets: set[str] = set()
    library = manifest.get("lib")
    if isinstance(library, dict) and isinstance(library.get("path"), str):
        targets.add(library["path"])
    elif package.get("name"):
        targets.add("src/lib.rs")
    for target_kind in ("bin", "example", "test", "bench"):
        configured = manifest.get(target_kind, [])
        if isinstance(configured, dict):
            configured = [configured]
        for target in configured:
            if isinstance(target, dict) and isinstance(target.get("path"), str):
                targets.add(target["path"])
    if not manifest.get("bin") and package.get("name"):
        targets.add("src/main.rs")
    return targets


def main() -> int:
    if len(sys.argv) != 3:
        raise RuntimeError("usage: build-context.py WORKSPACE_ROOT EMPTY_CONTEXT")
    root = Path(sys.argv[1]).resolve(strict=True)
    destination = Path(sys.argv[2]).resolve(strict=True)
    if any(destination.iterdir()):
        raise RuntimeError(f"build context must be empty: {destination}")

    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    guest_packages = guest_dependency_closure(root, workspace)
    actual_guest_packages = {path.as_posix() for path in guest_packages}
    if actual_guest_packages != EXPECTED_GUEST_PACKAGES:
        raise RuntimeError(
            "guest workspace dependency set changed; update the explicit Dockerfile COPY list: "
            + ",".join(sorted(actual_guest_packages))
        )

    for input_path in ROOT_INPUTS:
        copy_file(root, Path(input_path), destination)

    all_packages = workspace_package_dirs(root, workspace)
    if not guest_packages.issubset(all_packages):
        raise RuntimeError("guest dependency closure contains a package outside workspace.members")
    for package_dir in sorted(all_packages):
        manifest_path = root / package_dir / "Cargo.toml"
        copy_file(root, manifest_path.relative_to(root), destination)
        package_manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        build_script = package_manifest.get("package", {}).get("build")
        if build_script is False:
            continue
        build_script = build_script if isinstance(build_script, str) else "build.rs"
        source = root / package_dir / build_script
        if source.is_file():
            copy_file(root, source.relative_to(root), destination)
        if package_dir in guest_packages:
            for name in PACKAGE_INPUTS:
                source = root / package_dir / name
                if source.is_dir():
                    copy_tree(root, source.relative_to(root), destination)
        else:
            for target in target_entrypoints(package_manifest):
                source = root / package_dir / target
                if source.is_file():
                    copy_file(root, source.relative_to(root), destination)

    copy_guest_build_support(root, guest_packages, destination)
    vendored_packages = copy_workspace_vendor_packages(root, workspace, destination)

    print("guest_packages=" + ",".join(sorted(actual_guest_packages)))
    print("workspace_manifests=" + str(len(all_packages)))
    print("vendored_path_packages=" + ",".join(sorted(path.as_posix() for path in vendored_packages)))
    copied = sorted(
        path.relative_to(destination).as_posix()
        for path in destination.rglob("*")
        if path.is_file()
    )
    print("context_files=" + str(len(copied)))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, tomllib.TOMLDecodeError) as error:
        print(f"build-context: {error}", file=sys.stderr)
        raise SystemExit(1) from error
