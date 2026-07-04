import hashlib
import importlib.util
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[2]


def load_backend():
    path = ROOT / "packaging" / "agentrust_build_backend.py"
    spec = importlib.util.spec_from_file_location("agentrust_build_backend", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class BuildBackendSmokeTest(unittest.TestCase):
    def build_artifacts(self):
        backend = load_backend()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            wheel_name = backend.build_wheel(str(out))
            sdist_name = backend.build_sdist(str(out))
            wheel = out / wheel_name
            sdist = out / sdist_name
            return wheel_name, wheel.read_bytes(), sdist_name, sdist.read_bytes()

    def test_wheel_contains_only_public_package_surface(self):
        wheel_name, wheel_bytes, _, _ = self.build_artifacts()

        self.assertEqual("spendguard_agentrust-0.1.0a1-py3-none-any.whl", wheel_name)
        with tempfile.TemporaryDirectory() as tmp:
            wheel = Path(tmp) / wheel_name
            wheel.write_bytes(wheel_bytes)
            with zipfile.ZipFile(wheel) as archive:
                names = archive.namelist()
                self.assertEqual(sorted(names), names)
                self.assertIn("spendguard_agentrust/__init__.py", names)
                self.assertIn("spendguard_agentrust/__main__.py", names)
                self.assertIn("spendguard_agentrust/cli.py", names)
                self.assertIn("spendguard_agentrust/diagnostics.py", names)
                self.assertIn(
                    "spendguard_agentrust-0.1.0a1.dist-info/METADATA",
                    names,
                )
                self.assertIn(
                    "spendguard_agentrust-0.1.0a1.dist-info/RECORD",
                    names,
                )
                self.assertFalse(any("tests/" in name for name in names))
                self.assertFalse(any("docs/specs" in name for name in names))
                self.assertFalse(any("_verify_internal" in name for name in names))
                metadata = archive.read(
                    "spendguard_agentrust-0.1.0a1.dist-info/METADATA"
                ).decode("utf-8")

        self.assertIn("Name: spendguard-agentrust\n", metadata)
        self.assertIn("Version: 0.1.0a1\n", metadata)
        self.assertIn("Requires-Python: >=3.12\n", metadata)
        self.assertIn("Requires-Dist: agent-manifest==0.1.1\n", metadata)
        self.assertIn("Requires-Dist: agentrust-trace==0.2.0\n", metadata)
        self.assertIn("Requires-Dist: cryptography==49.0.0\n", metadata)
        self.assertIn("Requires-Dist: jsonschema==4.26.0\n", metadata)

    def test_sdist_contains_pyproject_backend_and_public_package(self):
        _, _, sdist_name, sdist_bytes = self.build_artifacts()

        self.assertEqual("spendguard_agentrust-0.1.0a1.tar.gz", sdist_name)
        with tempfile.TemporaryDirectory() as tmp:
            sdist = Path(tmp) / sdist_name
            sdist.write_bytes(sdist_bytes)
            with tarfile.open(sdist, "r:gz") as archive:
                names = archive.getnames()

        root = "spendguard_agentrust-0.1.0a1"
        self.assertEqual(sorted(names), names)
        self.assertIn(f"{root}/pyproject.toml", names)
        self.assertIn(f"{root}/packaging/agentrust_build_backend.py", names)
        self.assertIn(f"{root}/spendguard_agentrust/cli.py", names)
        self.assertIn(f"{root}/spendguard_agentrust/diagnostics.py", names)
        self.assertFalse(any("tests/" in name for name in names))
        self.assertFalse(any("docs/specs" in name for name in names))
        self.assertFalse(any("_verify_internal" in name for name in names))

    def test_artifacts_are_reproducible(self):
        first = self.build_artifacts()
        second = self.build_artifacts()

        self.assertEqual(first[0], second[0])
        self.assertEqual(first[2], second[2])
        self.assertEqual(hashlib.sha256(first[1]).hexdigest(), hashlib.sha256(second[1]).hexdigest())
        self.assertEqual(hashlib.sha256(first[3]).hexdigest(), hashlib.sha256(second[3]).hexdigest())


if __name__ == "__main__":
    unittest.main()
