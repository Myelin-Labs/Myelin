"""Build-kit boundary tests; no containers, network or executable archives."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import subprocess
import sys

import myelin_reproducible_build as build


class BuildKitBoundary(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        # Exercise parent aliases on every OS, including macOS /var -> /private/var.
        source = self.root / "actual"
        source.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(source, target_is_directory=True)
        self.root = alias

    def kit(self):
        source = self.root / "source"
        source.mkdir()
        (source / "file").write_bytes(b"declared source")
        records = build.archive(source, ["file"], self.root / "source.tar")
        build.archive(source, [], self.root / "vendor.tar")
        (self.root / "toolchain.oci.tar").write_bytes(b"opaque toolchain; input identity only, never executed")
        (self.root / "build.sh").write_bytes(b"exit 1\n")
        manifest = {
            "schema": build.SCHEMA,
            "image_id": "11" * 32,
            "artifacts": {name: {"bytes": (self.root / name).stat().st_size, "sha256": build.sha(self.root / name)} for name in build.ARCHIVES},
            "trees": {"source.tar": records, "vendor.tar": {}},
        }
        build.write_json(self.root / "manifest.json", manifest)
        return build.sha(self.root / "manifest.json")

    def test_external_recognition_and_complete_artifact_integrity(self):
        expected = self.kit()
        build.verify_inputs(self.root, expected)
        with self.assertRaisesRegex(ValueError, "unrecognized"):
            build.verify_inputs(self.root, "00" * 32)
        (self.root / "source.tar").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "changed or oversized"):
            build.verify_inputs(self.root, expected)
        (self.root / "source.tar").unlink()
        with self.assertRaises(FileNotFoundError):
            build.verify_inputs(self.root, expected)

    def test_tar_paths_links_duplicates_and_missing_members_reject(self):
        source = self.root / "source"
        source.mkdir()
        (self.root / "outside").mkdir()
        (self.root / "outside/file").write_bytes(b"outside")
        (source / "escape").symlink_to(self.root / "outside", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "source escapes its package"):
            build.archive(source, ["escape/file"], self.root / "escape.tar")
        for case in ("parent", "absolute", "symlink", "hardlink", "duplicate", "missing"):
            with self.subTest(case=case):
                name = {"parent": "../escape", "absolute": "/escape"}.get(case, "file")
                path = self.root / (case + ".tar")
                with tarfile.open(path, "w") as package:
                    for _ in range(0 if case == "missing" else 2 if case == "duplicate" else 1):
                        member = tarfile.TarInfo(name)
                        member.size = 1
                        member.mode = 0o644
                        if case in ("symlink", "hardlink"):
                            member.type = tarfile.SYMTYPE if case == "symlink" else tarfile.LNKTYPE
                            member.linkname = "/outside"
                            member.size = 0
                        package.addfile(member, io.BytesIO(b"x"))
                records = {name: {"bytes": 1, "mode": 0o644, "sha256": hashlib.sha256(b"x").hexdigest()}}
                with self.assertRaises(ValueError):
                    build.check_archive(path, records)

    def test_recognized_manifest_cannot_hide_member_mismatch_or_unknown_schema(self):
        self.kit()
        path = self.root / "manifest.json"
        manifest = json.loads(path.read_text())
        manifest["trees"]["source.tar"]["file"]["sha256"] = "00" * 32
        build.write_json(path, manifest)
        with self.assertRaisesRegex(ValueError, "content mismatch"):
            build.verify_inputs(self.root, build.sha(path))
        manifest["schema"] = "unknown-build-format"
        build.write_json(path, manifest)
        with self.assertRaisesRegex(ValueError, "unsupported"):
            build.verify_inputs(self.root, build.sha(path))

    def test_returned_binaries_and_runtime_files_must_match_inventory(self):
        for name in build.BINARIES:
            (self.root / name).write_bytes(name.encode())
        files = {name: {"bytes": (self.root / name).stat().st_size, "sha256": build.sha(self.root / name)} for name in build.BINARIES}
        build.write_json(self.root / "outputs.json", {"target": "fixture", "files": files})
        build.check_outputs(self.root)
        (self.root / "undeclared-library").write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "incomplete"):
            build.check_outputs(self.root)
        (self.root / "undeclared-library").unlink()
        (self.root / build.BINARIES[0]).write_bytes(b"wrong binary")
        with self.assertRaisesRegex(ValueError, "output mismatch"):
            build.check_outputs(self.root)

    def test_cli_rejects_unknown_trust_as_json_before_any_container_command(self):
        self.kit()
        process = subprocess.run([sys.executable, build.__file__, "verify-inputs", "--kit", str(self.root),
                                  "--expected-manifest", "00" * 32, "--podman", "/missing-must-not-be-invoked"], capture_output=True)
        self.assertEqual(process.returncode, 2)
        self.assertEqual(json.loads(process.stderr)["code"], "unknown-trust-anchor")

    def test_cli_reports_recognized_but_malformed_manifest_as_format_error(self):
        path = self.root / "manifest.json"
        path.write_text("{}")
        process = subprocess.run([sys.executable, build.__file__, "verify-inputs", "--kit", str(self.root),
                                  "--expected-manifest", build.sha(path), "--podman", "/missing-must-not-be-invoked"], capture_output=True)
        self.assertEqual(process.returncode, 2)
        self.assertEqual(json.loads(process.stderr)["code"], "format")

    def test_partial_or_different_builds_cannot_be_reported_as_reproduced(self):
        manifest = {"image_id": "11" * 32, "artifacts": {}}
        with self.assertRaisesRegex(ValueError, "two independent"):
            build.record_reproduction(self.root, manifest, "22" * 32, [{}])
        self.assertFalse((self.root / "reproduction.json").exists())
        with self.assertRaisesRegex(RuntimeError, "builds differ"):
            build.record_reproduction(self.root, manifest, "22" * 32, [{"binary": "old"}, {"binary": "changed"}])
        self.assertEqual(json.loads((self.root / "reproduction.json").read_text())["status"], "mismatch")


if __name__ == "__main__":
    unittest.main()
