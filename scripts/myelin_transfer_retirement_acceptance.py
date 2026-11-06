#!/usr/bin/env python3
"""Run the transfer example through lifecycle and continuation process restarts."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    cases = []
    for mode in ["exact", "policy"]:
        with tempfile.TemporaryDirectory(prefix="myelin-retirement-") as tmp:
            root = Path(tmp)
            calls = []

            def run(op, name="transfer", kind="scarce", accepted=True):
                command = [str(binary), op, str(root / name), kind, mode]
                result = subprocess.run(command, capture_output=True, text=True, check=False)
                calls.append({"arguments": command[1:], "exit_code": result.returncode,
                              "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(),
                              "stderr": result.stderr})
                assert (result.returncode == 0) == accepted, (command, result.stderr)
                return json.loads(result.stdout) if accepted else None

            issued = run("issue")
            assert issued["consumed_by"] is None
            assert issued["sessions"][0]["locked"] == 10
            assert issued["capabilities"]["consumed_lock_retirement"]
            run("retire", accepted=False)
            assert run("replay") == issued
            consumed = run("resume")
            assert consumed["sessions"][0]["locked"] == 10
            assert consumed["sessions"][1]["available"] == 10
            continued = run("continue")
            assert continued["sessions"][0]["sealed_by_successor"] == continued["active_source_session"]
            retired = run("retire")
            assert "Retired" in retired["lifecycle"]["terminal"]
            original, target, source = retired["sessions"]
            assert (source["available"], source["locked"], source["pending_locks"], source["height"]) == (90, 0, 0, 0)
            assert (original["locked"], original["height"]) == (10, 1)
            assert (target["available"], target["received_receipts"], target["height"]) == (10, 1, 0)
            assert source["pending_outbox"] == 1
            for op in ["retire", "resume", "replay"]:
                assert run(op) == retired
            for op in ["cancel", "refund"]:
                run(op, accepted=False)
            copyable = run("resume", name="copy", kind="copyable")
            assert not copyable["capabilities"]["consumed_lock_retirement"]
            run("retire", name="copy", kind="copyable", accepted=False)
            assert run("replay", name="copy") == copyable
            pending = run("issue", name="inherited")
            run("unsupported-upgrade", name="inherited", accepted=False)
            assert run("replay", name="inherited") == pending
            continued = run("continue", name="inherited")
            assert run("continue", name="inherited") == continued
            cancelled = run("cancel", name="inherited")
            assert cancelled["lifecycle"]["terminal"]["Cancelled"]["at"]["session_id"] == [5] * 32
            refunded = run("refund", name="inherited")
            assert (refunded["sessions"][2]["available"], refunded["sessions"][2]["locked"]) == (100, 0)
            assert run("replay", name="inherited") == refunded
            cases.append({"target_mode": mode, "retired_result": retired, "calls": calls})
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == binary_hash, "binary changed during acceptance"
    args.output.write_text(json.dumps({"binary": str(binary), "binary_sha256": binary_hash,
                                      "cases": cases, "passed": True}, indent=2) + "\n")
    print(json.dumps({"passed": True, "target_modes": len(cases), "process_invocations": sum(len(c["calls"]) for c in cases)}))


if __name__ == "__main__":
    main()
