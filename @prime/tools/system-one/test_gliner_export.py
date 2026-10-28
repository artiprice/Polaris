import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("gliner_export.py")
SPEC = importlib.util.spec_from_file_location("gliner_export", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load GLiNER export pipeline")
PIPELINE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PIPELINE)


class ExportPipelineTests(unittest.TestCase):
    def test_config_rejects_wrong_checkpoint_revision_and_architecture(self) -> None:
        config = json.loads(PIPELINE.CONFIG_PATH.read_text(encoding="utf-8"))
        config["checkpoint"]["revision"] = "other"
        with tempfile.TemporaryDirectory() as temporary:
            config_path = Path(temporary) / "config.json"
            config_path.write_text(json.dumps(config), encoding="utf-8")
            with self.assertRaisesRegex(PIPELINE.PipelineError, "revision"):
                PIPELINE.load_config(config_path)
        config["checkpoint"]["revision"] = "9d1bfb848cd16d93ee56ccbc59cd92bf0f284d90"
        config["checkpoint"]["architecture"] = "boundary"
        with tempfile.TemporaryDirectory() as temporary:
            config_path = Path(temporary) / "config.json"
            config_path.write_text(json.dumps(config), encoding="utf-8")
            with self.assertRaisesRegex(PIPELINE.PipelineError, "architecture"):
                PIPELINE.load_config(config_path)

    def test_pinned_tool_hash_mismatch_blocks_export_and_goldens(self) -> None:
        config = json.loads(PIPELINE.CONFIG_PATH.read_text(encoding="utf-8"))
        config["tools"]["exporter"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(PIPELINE.PipelineError, "bytes changed"):
            PIPELINE.pinned_tools(config)

    def test_export_manifest_requires_all_upstream_fragment_variants(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for name in PIPELINE.expected_export_files() - {"classifier_fp16_iobinding.onnx"}:
                (output / name).write_bytes(b"realistic-test-fixture")
            with self.assertRaisesRegex(PIPELINE.PipelineError, "omitted required files"):
                PIPELINE.validate_export_tree(output)

    def test_golden_manifest_requires_exact_seeded_ids_and_category_counts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            inputs = []
            for index in range(200):
                kind = "choice" if index < 60 else "noul" if index < 120 else "score" if index < 170 else "multi3"
                inputs.append({"id": f"g{index:03d}", "kind": kind, "schema": {"intent": ["yes", "no"]}})
            reference = [
                {
                    "id": row["id"],
                    "kind": row["kind"],
                    "outputs": {"intent": {"label": "yes", "probs": {"yes": 0.75, "no": 0.25}}},
                }
                for row in inputs
            ]
            (output / "golden_inputs.jsonl").write_text(
                "".join(json.dumps(row) + "\n" for row in inputs), encoding="utf-8"
            )
            (output / "golden_python.jsonl").write_text(
                "".join(json.dumps(row) + "\n" for row in reference), encoding="utf-8"
            )
            checked = PIPELINE.validate_golden_tree(output)
            self.assertEqual([path.name for path in checked], ["golden_inputs.jsonl", "golden_python.jsonl"])
            reference[0]["outputs"] = {}
            (output / "golden_python.jsonl").write_text(
                "".join(json.dumps(row) + "\n" for row in reference), encoding="utf-8"
            )
            with self.assertRaisesRegex(PIPELINE.PipelineError, "reference outputs are incomplete"):
                PIPELINE.validate_golden_tree(output)
            reference[0]["outputs"] = {"intent": {"label": "no", "probs": {"yes": 0.75, "no": 0.25}}}
            (output / "golden_python.jsonl").write_text(
                "".join(json.dumps(row) + "\n" for row in reference), encoding="utf-8"
            )
            with self.assertRaisesRegex(PIPELINE.PipelineError, "labels do not match"):
                PIPELINE.validate_golden_tree(output)

    def test_normalization_matches_python_gliner_period_contract(self) -> None:
        cases = {"": ".", "hello": "hello.", "already.": "already.", "question?": "question?", "alert!": "alert!", "unicode🙂": "unicode🙂."}
        for source, expected in cases.items():
            with self.subTest(source=source):
                self.assertEqual(PIPELINE.append_python_period(source), expected)

    def test_provenance_hashes_actual_files_and_rejects_root_escape(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "root"
            root.mkdir()
            artifact = root / "artifact.bin"
            artifact.write_bytes(b"bytes from a test fixture")
            record = PIPELINE.file_record(artifact, root)
            self.assertEqual(record["size_bytes"], artifact.stat().st_size)
            self.assertEqual(record["sha256"], PIPELINE.sha256_file(artifact))
            elsewhere = Path(temporary) / "elsewhere"
            elsewhere.mkdir()
            outside = elsewhere / "outside.bin"
            outside.write_bytes(b"outside")
            with self.assertRaisesRegex(PIPELINE.PipelineError, "escaped"):
                PIPELINE.file_record(outside, root)

    def test_export_memory_gate_does_not_start_or_create_output(self) -> None:
        config = json.loads(PIPELINE.CONFIG_PATH.read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "new-export"
            with patch.object(PIPELINE, "free_memory_bytes", return_value=PIPELINE.MIN_FREE_MEMORY_BYTES - 1):
                with self.assertRaisesRegex(PIPELINE.PipelineError, "insufficient available RAM"):
                    PIPELINE.run_export(config, target, Path(sys.executable), PIPELINE.MIN_FREE_MEMORY_BYTES)
            self.assertFalse(target.exists())

    def test_output_planning_allows_missing_nested_parent_without_creating_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "not-created" / "nested" / "output"
            planned = PIPELINE.validate_new_output_target(target)
            self.assertEqual(planned.name, "output")
            self.assertFalse(target.parent.exists())
            target.mkdir(parents=True)
            with self.assertRaisesRegex(PIPELINE.PipelineError, "refusing to overwrite"):
                PIPELINE.validate_new_output_target(target)

    def test_golden_inference_memory_gate_does_not_create_target(self) -> None:
        config = json.loads(PIPELINE.CONFIG_PATH.read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "new-goldens"
            with patch.object(PIPELINE, "free_memory_bytes", return_value=PIPELINE.MIN_FREE_MEMORY_BYTES - 1):
                with self.assertRaisesRegex(PIPELINE.PipelineError, "insufficient available RAM for golden inference"):
                    PIPELINE.run_goldens(config, target, Path(sys.executable))
            self.assertFalse(target.exists())

    def test_config_requires_complete_named_tool_manifest(self) -> None:
        config = json.loads(PIPELINE.CONFIG_PATH.read_text(encoding="utf-8"))
        config["tools"].pop("python_reference")
        with tempfile.TemporaryDirectory() as temporary:
            config_path = Path(temporary) / "config.json"
            config_path.write_text(json.dumps(config), encoding="utf-8")
            with self.assertRaisesRegex(PIPELINE.PipelineError, "tool manifest"):
                PIPELINE.load_config(config_path)


if __name__ == "__main__":
    unittest.main()
