"""Regression for the generator's implemented-package ownership boundary."""
import importlib.util
import pathlib
import tempfile
import unittest
from unittest.mock import patch

MODULE_PATH = pathlib.Path(__file__).resolve().parents[1] / 'workspace.py'

class WorkspaceOwnershipTests(unittest.TestCase):
    def test_generation_preserves_implemented_test_package(self):
        spec = importlib.util.spec_from_file_location('workspace_ownership_fixture', MODULE_PATH)
        self.assertIsNotNone(spec)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            files = {'tests/Cargo.toml': '# '+module.MARKER+'\n[package]\nname="owned"\n',
                     'tests/src/lib.rs': '// '+module.MARKER+'\n//! Implemented owner.\n'}
            for name, content in files.items():
                path = root/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding='utf-8')
            with patch.object(module, 'ROOT', root), patch.object(module, 'GENERATED_ROOTS', []), patch.object(module, 'load_table', return_value={'crate':[{'path':'tests','implemented':True}]}):
                module.remove_generated()
            for name, content in files.items():
                self.assertTrue((root/name).is_file(), name+' belongs to its implemented package')
                self.assertEqual((root/name).read_text(encoding='utf-8'), content)

if __name__ == '__main__':
    unittest.main()
