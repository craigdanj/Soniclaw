import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
SPEC = importlib.util.spec_from_file_location("suite", Path(__file__).parents[1] / "compatibility-suite.py")
suite = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(suite)


class CompatibilityTests(unittest.TestCase):
    def archive(self, root, content=b"source", corrupt=False, extra=None):
        path = root / "source.zip"
        files = {"a.rs": hashlib.sha256(content).hexdigest()}
        with zipfile.ZipFile(path, "w") as z:
            z.writestr("soniclaw/a.rs", b"corrupt" if corrupt else content)
            z.writestr("soniclaw/SOURCE_MANIFEST.json", json.dumps({"version": "0.146.0", "files_sha256": files}))
            if extra:
                z.writestr(extra, "extra")
        return path

    def test_archive_detects_corruption_and_extra_files(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.assertEqual(suite.verify_archive(self.archive(root))["files"], 1)
            with self.assertRaises(ValueError):
                suite.verify_archive(self.archive(root, corrupt=True))
            with self.assertRaises(ValueError):
                suite.verify_archive(self.archive(root, extra="soniclaw/unlisted.rs"))

    def test_archive_rejects_traversal(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                suite.verify_archive(self.archive(Path(d), extra="../escape"))

    def test_acceptance_cannot_claim_unrun_cases_passed(self):
        doc = suite.acceptance_template()
        doc["platform"] = "linux"
        self.assertEqual(suite.validate_acceptance(doc)["status"], "pending")
        doc["cases"][0]["status"] = "passed"
        with self.assertRaises(ValueError):
            suite.validate_acceptance(doc)
        doc.update(tester="Operator", tested_at="2026-11-17")
        doc["environment"]["os"] = "Linux"
        for case in doc["cases"]:
            case.update(status="passed", evidence="operator-session.log")
        self.assertEqual(suite.validate_acceptance(doc)["status"], "passed")
        doc["cases"][0]["status"] = "failed"
        self.assertEqual(suite.validate_acceptance(doc)["status"], "failed")

    def test_acceptance_duplicate_case_is_rejected(self):
        doc = suite.acceptance_template()
        doc["platform"] = "linux"
        doc["cases"][1] = doc["cases"][0].copy()
        with self.assertRaises(ValueError):
            suite.validate_acceptance(doc)

    def test_child_timeout_and_nonzero_are_failures(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            runner = suite.Runner(d, timeout=0.05)
            self.assertEqual(runner.command("bad-exit", [sys.executable, "-c", "raise SystemExit(7)"])["status"], "failed")
            self.assertEqual(runner.command("timeout", [sys.executable, "-c", "import time;time.sleep(1)"])["status"], "failed")
            report = json.loads((Path(d) / "report.json").read_text())
            self.assertEqual(report["automated_status"], "failed")
            self.assertEqual(report["acceptance_status"], "pending")

    def test_source_version_mismatch_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion="0.146.0"\n')
            crate = root / "crates/test"
            crate.mkdir(parents=True)
            (crate / "Cargo.toml").write_text('[package]\nname="soniclaw-test"\nversion="0.14.0"\n')
            with self.assertRaisesRegex(ValueError, "stale package"):
                suite.source_contract(root)


if __name__ == "__main__":
    unittest.main()

class BenchmarkInputTests(unittest.TestCase):
    def test_nonfinite_timing_is_rejected(self):
        spec = importlib.util.spec_from_file_location("compare", Path(__file__).parents[1] / "compare-benchmarks.py")
        compare = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(compare)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "bad.csv"
            path.write_text("plugin,mode,frames,cpu_ns_per_callback\nx,0,256,NaN\n")
            with self.assertRaises(ValueError):
                compare.load(path)

    def test_gate_rejects_missing_workloads(self):
        import subprocess
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            header = "plugin,mode,frames,cpu_ns_per_callback\n"
            (root / "baseline.csv").write_text(header + "x,0,256,10\nx,1,256,12\n")
            (root / "current.csv").write_text(header + "x,0,256,9\n")
            result = subprocess.run([sys.executable, str(Path(__file__).parents[1] / "compare-benchmarks.py"), str(root / "baseline.csv"), str(root / "current.csv"), "--output", str(root / "out"), "--fail-regression-percent", "10"], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("identical benchmark cells", result.stderr)
