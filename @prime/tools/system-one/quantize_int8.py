"""Dynamic int8 quantization of an exported GLiNER2 span checkpoint (offline tool).

Only the DeBERTa encoder's MatMul weights are quantized (QInt8, per-tensor); the span, count,
scorer and classifier heads stay fp32. Dynamic quantization keeps fp32 graph inputs/outputs, so
the result loads through gliner2-rs `Precision::Fp32` under the same fragment file names. The
written `manifest.json` records the real precision, tool versions and SHA-256 of every file.

Usage (repository root, S0-4 environment):
    python -X utf8 -P @prime/tools/system-one/quantize_int8.py <fp32-dir> <new-int8-dir>
        [--per-channel] [--include-gather]

`--include-gather` also quantizes embedding Gather tables (smaller, larger accuracy risk).
The output directory must not exist; files are staged and promoted only after all succeed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

FRAGMENTS = (
    "classifier",
    "count_lstm_fixed",
    "count_pred_argmax",
    "encoder",
    "schema_gather",
    "scorer",
    "span_rep",
    "token_gather",
)


class QuantizeError(Exception):
    """Expected failure with a message safe to print."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_source(source: Path) -> None:
    missing = [f"{name}_fp32.onnx" for name in FRAGMENTS if not (source / f"{name}_fp32.onnx").is_file()]
    if not (source / "tokenizer.json").is_file():
        missing.append("tokenizer.json")
    if missing:
        raise QuantizeError(f"source is not a complete fp32 export; missing: {', '.join(missing)}")


def weight_only(source_model: Path, target_model: Path, block_size: int, accuracy_level: int | None) -> None:
    """Block-wise symmetric 8-bit MatMulNBits weights; `accuracy_level` 4 computes in int8 with
    block-wise activation quantization, otherwise compute stays fp32."""
    from onnxruntime.quantization.matmul_nbits_quantizer import (
        DefaultWeightOnlyQuantConfig,
        MatMulNBitsQuantizer,
    )

    config = DefaultWeightOnlyQuantConfig(block_size=block_size, is_symmetric=True, bits=8)
    quantizer = MatMulNBitsQuantizer(
        str(source_model),
        bits=8,
        block_size=block_size,
        is_symmetric=True,
        accuracy_level=accuracy_level,
        algo_config=config,
    )
    quantizer.process()
    quantizer.model.save_model_to_file(str(target_model), False)


def ffn_matmuls(model_path: Path) -> list[str]:
    """Names of the feed-forward MatMuls (intermediate and output dense, not attention)."""
    import onnx

    graph = onnx.load(str(model_path), load_external_data=False).graph
    names = [
        node.name
        for node in graph.node
        if node.op_type == "MatMul"
        and ("/intermediate/dense/" in node.name or ("/output/dense/" in node.name and "/attention/" not in node.name))
    ]
    if not names:
        raise QuantizeError("no feed-forward MatMul nodes found; unexpected encoder naming")
    return names


def quantize(
    source: Path,
    output: Path,
    include_gather: bool,
    per_channel: bool,
    weight_only_block: int | None = None,
    accuracy_level: int | None = None,
    ffn_only: bool = False,
) -> dict:
    import onnx
    import onnxruntime
    from onnxruntime.quantization import QuantType, quantize_dynamic

    if output.exists():
        raise QuantizeError("output directory already exists; choose a new path")
    check_source(source)
    staging = output.with_name(output.name + ".staging")
    if staging.exists():
        raise QuantizeError(f"stale staging directory exists: {staging}")
    staging.mkdir(parents=True)
    try:
        for name in FRAGMENTS:
            if name != "encoder":
                shutil.copy2(source / f"{name}_fp32.onnx", staging / f"{name}_fp32.onnx")
        shutil.copy2(source / "tokenizer.json", staging / "tokenizer.json")
        for extra in source.glob("encoder_fp32.onnx.data"):
            raise QuantizeError(f"external-data encoders are not supported yet: {extra.name}")
        op_types = ["MatMul", "Gemm"] + (["Gather"] if include_gather else [])
        if weight_only_block is not None:
            op_types = ["MatMul"]
            weight_only(
                source / "encoder_fp32.onnx",
                staging / "encoder_fp32.onnx",
                weight_only_block,
                accuracy_level,
            )
        else:
            nodes = ffn_matmuls(source / "encoder_fp32.onnx") if ffn_only else None
            quantize_dynamic(
                model_input=str(source / "encoder_fp32.onnx"),
                model_output=str(staging / "encoder_fp32.onnx"),
                weight_type=QuantType.QInt8,
                per_channel=per_channel,
                reduce_range=False,
                op_types_to_quantize=op_types,
                nodes_to_quantize=nodes,
            )
        # onnx.checker rejects outer-scope references inside the exported `If` subgraphs after
        # quantization renames nodes; ONNX Runtime is the actual consumer, so it validates here.
        session_options = onnxruntime.SessionOptions()
        session_options.graph_optimization_level = onnxruntime.GraphOptimizationLevel.ORT_DISABLE_ALL
        onnxruntime.InferenceSession(
            str(staging / "encoder_fp32.onnx"),
            sess_options=session_options,
            providers=["CPUExecutionProvider"],
        )
        manifest = {
            "schema_version": 1,
            "precision": (
                f"int8-nbits-block{weight_only_block}-acc{accuracy_level or 0}-encoder"
                if weight_only_block is not None
                else "int8-dynamic-encoder"
                + ("-ffn-only" if ffn_only else "")
                + ("-per-channel" if per_channel else "")
                + ("+gather" if include_gather else "")
            ),
            "file_naming": "gliner2-rs Precision::Fp32 names; graph I/O stays fp32",
            "quantized_fragments": ["encoder"],
            "op_types": op_types,
            "weight_type": "QInt8",
            "per_channel": per_channel,
            "weight_only_block_size": weight_only_block,
            "accuracy_level": accuracy_level,
            "source": {name: sha256(source / name) for name in sorted(p.name for p in source.iterdir() if p.is_file())},
            "files": {p.name: sha256(p) for p in sorted(staging.iterdir()) if p.is_file()},
            "tools": {
                "python": sys.version.split()[0],
                "onnx": onnx.__version__,
                "onnxruntime": onnxruntime.__version__,
            },
        }
        (staging / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        staging.rename(output)
        return manifest
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path, help="complete fp32 export directory")
    parser.add_argument("output", type=Path, help="new int8 directory (must not exist)")
    parser.add_argument("--include-gather", action="store_true", help="also quantize embedding tables")
    parser.add_argument("--per-channel", action="store_true", help="per-output-channel weight scales")
    parser.add_argument("--ffn-only", action="store_true", help="quantize only feed-forward MatMuls")
    parser.add_argument(
        "--weight-only-block",
        type=int,
        choices=(16, 32, 64, 128),
        help="8-bit block-wise weight-only MatMulNBits instead of dynamic activation quantization",
    )
    parser.add_argument(
        "--accuracy-level",
        type=int,
        choices=(1, 2, 3, 4),
        help="MatMulNBits compute type (4 = int8 compute with block-wise activations)",
    )
    args = parser.parse_args()
    try:
        manifest = quantize(
            args.source.resolve(),
            args.output.resolve(),
            args.include_gather,
            args.per_channel,
            args.weight_only_block,
            args.accuracy_level,
            args.ffn_only,
        )
    except QuantizeError as error:
        print(f"quantize_int8: {error}", file=sys.stderr)
        return 2
    sizes = {name: (args.output / name).stat().st_size for name in manifest["files"]}
    print(json.dumps({"output": str(args.output), "precision": manifest["precision"], "bytes": sizes}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
