#!/usr/bin/env python3
"""Reproducible, pinned GLiNER2.5-Decide ONNX and golden generation.

The tool invokes the vendored upstream span exporter and the S0-4 Python reference
scripts unchanged. It never downloads a checkpoint, writes only to a new output
directory, and records hashes for every model, tool, input, and generated artifact.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
SPIKE = ROOT / "@prime" / "spikes" / "s0-4-gliner"
CONFIG_PATH = ROOT / "@prime" / "tools" / "system-one" / "gliner_export_config.json"
DEFAULT_EXPORT_OUTPUT = Path.home() / ".polaris" / "models" / "gliner2.5-decide-onnx-p4"
DEFAULT_GOLDEN_OUTPUT = ROOT / "crates" / "polaris-system-one" / "crates" / "engine" / "tests" / "fixtures" / "s1" / "decide"
GIB = 1024**3
MIN_FREE_MEMORY_BYTES = 10 * GIB
FRAGMENTS = (
    "encoder",
    "token_gather",
    "span_rep",
    "schema_gather",
    "count_pred_argmax",
    "count_lstm_fixed",
    "scorer",
    "classifier",
)
EXPORT_VARIANTS = ("fp32", "fp16", "fp16_iobinding")


class PipelineError(Exception):
    """A sanitized validation or pipeline failure."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def file_record(path: Path, base: Path) -> dict[str, Any]:
    if path.is_symlink():
        raise PipelineError("provenance input cannot be a symbolic link")
    resolved = path.resolve(strict=True)
    try:
        relative = resolved.relative_to(base.resolve(strict=True))
    except ValueError as error:
        raise PipelineError("provenance input escaped its declared root") from error
    if not resolved.is_file() or resolved.is_symlink():
        raise PipelineError("provenance input must be a regular file")
    return {"path": relative.as_posix(), "size_bytes": resolved.stat().st_size, "sha256": sha256_file(resolved)}


def append_python_period(text: str) -> str:
    """Match gliner2 2.1.0 SchemaTransformer._normalize_text behavior."""
    if not text.endswith((".", "!", "?")):
        return text + "."
    return text


def load_config(path: Path = CONFIG_PATH) -> dict[str, Any]:
    try:
        config = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PipelineError("pipeline config is missing or invalid JSON") from error
    if config.get("schema_version") != 1:
        raise PipelineError("unsupported pipeline config schema")
    if not isinstance(config.get("tools"), dict) or not isinstance(config.get("checkpoint"), dict):
        raise PipelineError("pipeline config is missing checkpoint or tool pins")
    expected_tools = {"exporter", "golden_generator", "python_reference"}
    if set(config["tools"]) != expected_tools:
        raise PipelineError("pipeline tool manifest must contain exactly the pinned exporter and reference tools")
    if config.get("checkpoint", {}).get("repo") != "fastino/GLiNER2.5-Decide":
        raise PipelineError("checkpoint repository does not match the pinned S0-4 model")
    if config.get("checkpoint", {}).get("revision") != "9d1bfb848cd16d93ee56ccbc59cd92bf0f284d90":
        raise PipelineError("checkpoint revision does not match the pinned S0-4 model")
    if config.get("checkpoint", {}).get("architecture") != "span":
        raise PipelineError("checkpoint architecture does not match the pinned span exporter")
    minimum_ram = config.get("resource_limits", {}).get("minimum_free_ram_gib")
    if not isinstance(minimum_ram, int) or minimum_ram < 10:
        raise PipelineError("pipeline RAM requirement must be an integer of at least 10 GiB")
    return config


def resolve_inside(root: Path, relative: str) -> Path:
    candidate = (root / relative).resolve(strict=True)
    try:
        candidate.relative_to(root.resolve(strict=True))
    except ValueError as error:
        raise PipelineError("configured path escapes its trusted root") from error
    return candidate


def cached_checkpoint(config: dict[str, Any]) -> tuple[Path, Path]:
    hf_home = Path(os.environ.get("HF_HOME", Path.home() / ".polaris" / "models" / "hf"))
    snapshots = hf_home / "hub" / "models--fastino--GLiNER2.5-Decide" / "snapshots"
    revision = config["checkpoint"]["revision"]
    snapshot = snapshots / revision
    if not snapshot.is_dir():
        raise PipelineError(f"pinned checkpoint snapshot is not cached: {revision}")
    required = ("config.json", "model.safetensors", "tokenizer.json")
    for name in required:
        path = snapshot / name
        if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
            raise PipelineError(f"pinned checkpoint is incomplete: {name}")
    try:
        model_config = json.loads((snapshot / "config.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PipelineError("checkpoint config is invalid") from error
    if model_config.get("architecture") != "span":
        raise PipelineError("pinned checkpoint is not a span architecture")
    return hf_home.resolve(), snapshot.resolve()


def pinned_tools(config: dict[str, Any]) -> dict[str, Path]:
    tools: dict[str, Path] = {}
    for name, specification in config["tools"].items():
        root_key = specification["root"]
        root = ROOT if root_key == "workspace" else SPIKE
        path = resolve_inside(root, specification["path"])
        if path.name.lower() in {".env", "credentials", "credentials.toml"}:
            raise PipelineError("sensitive path cannot be used as a pipeline tool")
        observed = sha256_file(path)
        if observed != specification["sha256"]:
            raise PipelineError(f"pinned tool bytes changed: {name}")
        tools[name] = path
    return tools


def environment_versions(python: Path, cwd: Path) -> dict[str, str]:
    code = (
        "import gliner2, numpy, onnx, onnxscript, onnxruntime, psutil, torch, transformers; "
        "print(__import__('json').dumps({"
        "'gliner2':getattr(gliner2,'__version__','unknown'), 'numpy':numpy.__version__,"
        "'onnx':onnx.__version__, 'onnxscript':onnxscript.__version__,"
        "'onnxruntime':onnxruntime.__version__, 'psutil':psutil.__version__,"
        "'torch':torch.__version__, 'transformers':transformers.__version__}))"
    )
    completed = subprocess.run(
        [str(python), "-X", "utf8", "-c", code],
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        raise PipelineError("pinned Python environment is not importable")
    try:
        versions = json.loads(completed.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise PipelineError("could not parse Python environment versions") from error
    return {"python": sys.version.split()[0], **versions}


def free_memory_bytes() -> int:
    try:
        import psutil
    except ImportError as error:
        raise PipelineError("psutil is required for an export memory preflight") from error
    return int(psutil.virtual_memory().available)


def expected_export_files() -> set[str]:
    return {f"{fragment}_{variant}.onnx" for fragment in FRAGMENTS for variant in EXPORT_VARIANTS} | {"tokenizer.json"}


def validate_export_tree(directory: Path) -> list[Path]:
    if not directory.is_dir():
        raise PipelineError("export output directory is missing")
    files = sorted(path for path in directory.iterdir() if path.is_file())
    names = {path.name for path in files}
    expected = expected_export_files()
    missing = sorted(expected - names)
    if missing:
        raise PipelineError("upstream exporter omitted required files: " + ",".join(missing))
    empty = [path.name for path in files if path.stat().st_size == 0]
    if empty:
        raise PipelineError("upstream exporter produced empty files: " + ",".join(empty))
    return files


def validate_golden_tree(directory: Path) -> tuple[Path, Path]:
    inputs = directory / "golden_inputs.jsonl"
    reference = directory / "golden_python.jsonl"
    try:
        input_rows = [json.loads(line) for line in inputs.read_text(encoding="utf-8").splitlines() if line]
        reference_rows = [json.loads(line) for line in reference.read_text(encoding="utf-8").splitlines() if line]
    except (OSError, json.JSONDecodeError) as error:
        raise PipelineError("golden files are missing or malformed") from error
    if len(input_rows) != 200 or len(reference_rows) != 200:
        raise PipelineError("golden generation must produce 200 input and reference rows")
    input_ids = [row.get("id") for row in input_rows]
    reference_ids = [row.get("id") for row in reference_rows]
    if input_ids != reference_ids or len(set(input_ids)) != 200:
        raise PipelineError("golden output IDs do not match the deterministic input set")
    kinds = {kind: sum(row.get("kind") == kind for row in input_rows) for kind in ("choice", "noul", "score", "multi3")}
    if kinds != {"choice": 60, "noul": 60, "score": 50, "multi3": 30}:
        raise PipelineError("golden input category counts differ from the S0-4 fixture contract")
    for input_row, reference_row in zip(input_rows, reference_rows, strict=True):
        schema = input_row.get("schema")
        outputs = reference_row.get("outputs")
        if (
            reference_row.get("kind") != input_row.get("kind")
            or not isinstance(schema, dict)
            or not isinstance(outputs, dict)
            or set(outputs) != set(schema)
        ):
            raise PipelineError("golden reference outputs are incomplete or mismatched")
        for task, labels in schema.items():
            result = outputs[task]
            if not isinstance(labels, list) or not isinstance(result, dict):
                raise PipelineError("golden reference outputs are incomplete or mismatched")
            probabilities = result.get("probs")
            label = result.get("label")
            if not isinstance(probabilities, dict) or set(probabilities) != set(labels) or label not in probabilities:
                raise PipelineError("golden reference outputs are incomplete or mismatched")
            values = list(probabilities.values())
            if (
                any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= 1 for value in values)
                or abs(math.fsum(values) - 1.0) > 1e-4
            ):
                raise PipelineError("golden reference probabilities are not a finite normalized distribution")
            if probabilities[label] != max(values):
                raise PipelineError("golden reference labels do not match their probability maxima")
    return inputs, reference


def validate_new_output_target(path: Path) -> Path:
    target = path.expanduser().absolute()
    if target.exists() or target.is_symlink():
        raise PipelineError("output target already exists; refusing to overwrite")
    missing_parents: list[str] = []
    ancestor = target.parent
    while not ancestor.exists():
        if ancestor == ancestor.parent:
            raise PipelineError("output path has no existing parent")
        missing_parents.append(ancestor.name)
        ancestor = ancestor.parent
    resolved_parent = ancestor.resolve(strict=True)
    if not resolved_parent.is_dir() or not os.access(resolved_parent, os.W_OK):
        raise PipelineError("nearest existing output parent must be writable")
    return resolved_parent.joinpath(*reversed(missing_parents), target.name)


def run_export(config: dict[str, Any], output: Path, python: Path, minimum_memory: int) -> None:
    output = validate_new_output_target(output)
    available = free_memory_bytes()
    if available < minimum_memory:
        raise PipelineError(f"insufficient available RAM for export: {available} bytes; need {minimum_memory}")
    hf_home, snapshot = cached_checkpoint(config)
    tools = pinned_tools(config)
    versions = environment_versions(python, SPIKE)
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".gliner-export-", dir=output.parent))
    env = dict(os.environ)
    env["HF_HOME"] = str(hf_home)
    env["HF_HUB_OFFLINE"] = "1"
    command = [
        str(python), "-X", "utf8", str(tools["exporter"]),
        "--model_path", str(snapshot), "--out_dir", str(staging),
    ]
    try:
        completed = subprocess.run(command, cwd=SPIKE, env=env, check=False)
        if completed.returncode != 0:
            raise PipelineError(f"upstream exporter exited with status {completed.returncode}")
        files = validate_export_tree(staging)
        provenance = {
            "schema_version": 1,
            "operation": "gliner2.5-decide-onnx-export",
            "checkpoint": {
                "repo": config["checkpoint"]["repo"],
                "revision": config["checkpoint"]["revision"],
                "files": [file_record((snapshot / name).resolve(strict=True), hf_home) for name in ("config.json", "model.safetensors", "tokenizer.json")],
            },
            "exporter": {
                "source": config["upstream"]["repository"],
                "revision": config["upstream"]["revision"],
                "script": file_record(tools["exporter"], ROOT),
                "command": command,
                "versions": versions,
            },
            "normalization": "Python gliner2 2.1.0 appends '.' unless input ends with '.', '!' or '?'.",
            "artifacts": [file_record(path, staging) for path in files],
        }
        (staging / "PROVENANCE.json").write_text(
            json.dumps(provenance, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        staging.rename(output)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def run_goldens(
    config: dict[str, Any], output: Path, python: Path, minimum_memory: int = MIN_FREE_MEMORY_BYTES
) -> None:
    output = validate_new_output_target(output)
    available = free_memory_bytes()
    if available < minimum_memory:
        raise PipelineError(f"insufficient available RAM for golden inference: {available} bytes; need {minimum_memory}")
    hf_home, snapshot = cached_checkpoint(config)
    tools = pinned_tools(config)
    versions = environment_versions(python, SPIKE)
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".gliner-goldens-", dir=output.parent))
    env = dict(os.environ)
    env["HF_HOME"] = str(hf_home)
    env["HF_HUB_OFFLINE"] = "1"
    try:
        for name in ("golden_generator", "python_reference"):
            completed = subprocess.run(
                [str(python), "-X", "utf8", "-P", str(tools[name])],
                cwd=staging,
                env=env,
                check=False,
            )
            if completed.returncode != 0:
                raise PipelineError(f"{name} exited with status {completed.returncode}")
        inputs, reference = validate_golden_tree(staging)
        provenance = {
            "schema_version": 1,
            "operation": "gliner2.5-decide-python-goldens",
            "fixture_kind": "deterministically generated synthetic inputs; labels in generator metadata are not model output or calibrated ground truth",
            "seed": 20261020,
            "checkpoint": {
                "repo": config["checkpoint"]["repo"],
                "revision": config["checkpoint"]["revision"],
                "files": [file_record((snapshot / name).resolve(strict=True), hf_home) for name in ("config.json", "model.safetensors", "tokenizer.json")],
            },
            "tools": {
                name: {"source": config["tools"][name]["path"], "sha256": sha256_file(tools[name])}
                for name in ("golden_generator", "python_reference")
            },
            "versions": versions,
            "normalization": "golden_python output is produced by the pinned Python gliner2 reference; downstream Rust parity must apply append_python_period semantics",
            "artifacts": [file_record(inputs, staging), file_record(reference, staging)],
        }
        (staging / "PROVENANCE.json").write_text(
            json.dumps(provenance, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        staging.rename(output)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=CONFIG_PATH)
    subparsers = parser.add_subparsers(dest="operation", required=True)
    validate = subparsers.add_parser("validate", help="check pinned local inputs without loading the model")
    validate.add_argument("--python", type=Path, default=SPIKE / ".venv" / "Scripts" / "python.exe")
    validate.add_argument("--export-output", type=Path, default=DEFAULT_EXPORT_OUTPUT)
    validate.add_argument("--golden-output", type=Path, default=DEFAULT_GOLDEN_OUTPUT)
    export = subparsers.add_parser("export", help="run the pinned upstream ONNX exporter into a new output directory")
    export.add_argument("output", type=Path)
    export.add_argument("--python", type=Path, default=SPIKE / ".venv" / "Scripts" / "python.exe")
    export.add_argument("--minimum-free-gib", type=int)
    goldens = subparsers.add_parser("goldens", help="generate pinned synthetic inputs and actual Python model outputs")
    goldens.add_argument("output", type=Path)
    goldens.add_argument("--python", type=Path, default=SPIKE / ".venv" / "Scripts" / "python.exe")
    args = parser.parse_args()
    try:
        config = load_config(args.config.resolve())
        tools = pinned_tools(config)
        hf_home, snapshot = cached_checkpoint(config)
        python = args.python.resolve(strict=True)
        if not python.is_file():
            raise PipelineError("configured Python interpreter is not a file")
        if args.operation == "validate":
            versions = environment_versions(python, SPIKE)
            memory = free_memory_bytes()
            export_target = validate_new_output_target(args.export_output)
            golden_target = validate_new_output_target(args.golden_output)
            output = {
                "status": "READY_INPUTS_BLOCKED_RAM" if memory < MIN_FREE_MEMORY_BYTES else "READY_FOR_EXPORT",
                "checkpoint_snapshot": str(snapshot),
                "model_repo": config["checkpoint"]["repo"],
                "model_revision": config["checkpoint"]["revision"],
                "exporter_sha256": sha256_file(tools["exporter"]),
                "available_ram_bytes": memory,
                "required_ram_bytes": MIN_FREE_MEMORY_BYTES,
                "export_memory_ready": memory >= MIN_FREE_MEMORY_BYTES,
                "hf_home": str(hf_home),
                "python_versions": versions,
                "outputs_must_be_new": True,
                "export_output": str(export_target),
                "golden_output": str(golden_target),
                "network_fetch_enabled": False,
            }
            print(json.dumps(output, ensure_ascii=False, sort_keys=True, indent=2))
            return 0 if memory >= MIN_FREE_MEMORY_BYTES else 3
        if args.operation == "export":
            minimum_free_gib = args.minimum_free_gib or config["resource_limits"]["minimum_free_ram_gib"]
            if minimum_free_gib < config["resource_limits"]["minimum_free_ram_gib"]:
                raise PipelineError("minimum export RAM cannot be set below 10 GiB")
            run_export(config, args.output, python, minimum_free_gib * GIB)
        else:
            run_goldens(config, args.output, python, config["resource_limits"]["minimum_free_ram_gib"] * GIB)
    except (PipelineError, OSError, subprocess.SubprocessError) as error:
        print(f"gliner-export: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
