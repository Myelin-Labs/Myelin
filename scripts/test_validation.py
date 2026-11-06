"""Exercise report failure boundaries with real subprocesses, without Cargo reruns."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import validation


class ValidationEvidence(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / "source"
        self.root.mkdir()
        self.out = Path(temporary.name) / "evidence"
        self.out.mkdir()
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "input").write_text("initial")
        subprocess.run(["git", "-C", str(self.root), "add", "input"], check=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"], check=True)

    def run_fixture(self, code, timeout=3, lanes=("core",)):
        run = validation.Run(self.root, self.out, list(lanes), "test-fixture", timeout)
        run.lane = lambda name: run.step(name, "fixture", [sys.executable, "-c", code])
        status = run.execute()
        return status, json.loads((self.out / "report.json").read_text())

    def test_selected_success_binds_source_logs_and_nested_reports_but_not_skipped_lanes(self):
        code = f"from pathlib import Path; p=Path({str(self.out / 'core/nested')!r}); p.mkdir(); (p/'report.json').write_text('evidence'); print('done')"
        scratch = self.out / validation.SCRATCH[0]
        scratch.mkdir(parents=True)
        os.mkfifo(scratch / "not-an-artifact")
        status, report = self.run_fixture(code)
        self.assertEqual(report["excluded_container_scratch"], [validation.SCRATCH[0]])
        self.assertEqual(status, 0)
        self.assertEqual(report["status"], "passed")
        self.assertFalse(report["source"]["dirty"])
        self.assertEqual(report["source"]["tree_sha256"], report["source_after"]["tree_sha256"])
        self.assertEqual(report["lanes"]["teeworlds"]["status"], "skipped")
        for name in ("core/fixture.log", "core/nested/report.json"):
            self.assertEqual(report["artifacts"][name]["sha256"], validation.sha(self.out / name))

    def test_failed_command_preserves_logs_and_cannot_pass_or_hide_unexecuted_required_lane(self):
        status, report = self.run_fixture("print('before failure'); raise SystemExit(7)", lanes=("core", "browser"))
        self.assertEqual(status, 1)
        self.assertEqual(report["lanes"]["core"]["steps"][0]["exit_code"], 7)
        self.assertEqual(report["lanes"]["core"]["status"], "failed")
        self.assertEqual(report["lanes"]["browser"]["status"], "skipped")
        self.assertIn("earlier failure", report["lanes"]["browser"]["reason"])
        self.assertFalse(report["selected_profile_passed"])
        self.assertIn("before failure", (self.out / "core/fixture.log").read_text())

    def test_timeout_reaps_child_and_source_mutation_cannot_be_reported_as_pass(self):
        status, report = self.run_fixture("import os,time; print(os.getpid(),flush=True); time.sleep(60)", timeout=1)
        self.assertEqual(status, 1)
        pid = int((self.out / "core/fixture.log").read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)
        self.assertEqual(report["lanes"]["core"]["steps"][0]["status"], "failed")
        self.out = self.out.parent / "changed-source"
        self.out.mkdir()
        status, report = self.run_fixture("from pathlib import Path; Path('input').write_text('changed')")
        self.assertEqual(status, 1)
        self.assertIn("source changed", report["error"])
        self.assertFalse(report["selected_profile_passed"])

    def test_missing_selected_tool_fails_with_machine_readable_evidence(self):
        run = validation.Run(self.root, self.out, ["browser"], "custom", 3)
        run.env.pop("MYELIN_BROWSER", None)
        self.assertEqual(run.execute(), 1)
        report = json.loads((self.out / "report.json").read_text())
        self.assertEqual(report["lanes"]["browser"]["status"], "failed")
        self.assertIn("MYELIN_BROWSER", report["lanes"]["browser"]["reason"])
        self.assertEqual(report["lanes"]["core"]["status"], "skipped")


if __name__ == "__main__":
    unittest.main()
