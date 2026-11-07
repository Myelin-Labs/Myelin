#!/usr/bin/env python3
"""Exercise shared-store lifecycle and independent-store receipt processes."""
import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile


def remote_receipt(binary):
    binary = binary.resolve(strict=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    calls = []
    with tempfile.TemporaryDirectory(prefix="myelin-remote-receipt-") as tmp:
        root = Path(tmp) / "deployment"
        wire = Path(tmp) / "relay.proof"

        def run(operation, proof=False, accepted=True, lose_reply=False):
            command = [str(binary), operation, str(root)] + ([str(wire)] if proof else [])
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=30)
            calls.append({"arguments": command[1:], "exit_code": result.returncode,
                          "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(), "stderr": result.stderr,
                          "reply_lost_before_client_observation": lose_reply})
            assert (result.returncode == 0) == accepted, (command, result.stderr)
            if lose_reply:
                # Retained by the evidence recorder, not used by the retry client.
                calls[-1]["lost_response_for_audit"] = result.stdout
                return None
            value = json.loads(result.stdout) if accepted else None
            calls[-1]["response"] = value
            return value

        initialized = run("init")
        assert initialized["different_authorities"]
        issued = run("issue")
        assert run("issue") == issued
        exported = run("export", proof=True)
        assert exported == issued
        wire_digest = hashlib.sha256(wire.read_bytes()).hexdigest()
        run("consume", proof=True, accepted=False)
        original_wire = wire.read_bytes()
        changed = bytearray(original_wire)
        changed[-1] ^= 1
        wire.write_bytes(changed)
        run("import", proof=True, accepted=False)
        wire.write_bytes(original_wire)
        # The source's actual RocksDB directory becomes unavailable. A fresh
        # source process now fails, while fresh target/relay processes continue.
        root.joinpath("source").rename(root / "source-unavailable")
        run("export", proof=True, accepted=False)
        assert hashlib.sha256(wire.read_bytes()).hexdigest() == wire_digest
        run("import", proof=True, lose_reply=True)
        imported = run("import", proof=True)
        assert not imported["inserted"] and not imported["consumed"]
        assert not imported["local_source_genesis"] and not imported["local_source_block"]
        run("consume", proof=True, lose_reply=True)
        lost_call = calls[-1]
        recovered = run("consume", proof=True)
        assert recovered == json.loads(lost_call["lost_response_for_audit"])
        assert recovered["receipts"] == 1
        assert run("consume", proof=True) == recovered
        redelivered = run("import", proof=True)
        assert not redelivered["inserted"] and redelivered["consumed"]
        replayed = run("replay")
        assert replayed["verified"] and replayed["state_root"] == recovered["state_root"]
        assert replayed["head"] == recovered["head"]
        assert root.joinpath("source-unavailable").is_dir() and not root.joinpath("source").exists()
        assert hashlib.sha256(wire.read_bytes()).hexdigest() == wire_digest
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    return {"binary": str(binary), "binary_sha256": digest, "proof_sha256": wire_digest,
            "scope": "copyable test receipt; file relay; independent native processes/stores/PoA authorities; no CKB or scarce assets",
            "original_result": recovered, "offline_replay": replayed, "calls": calls, "passed": True}


def terminal_transfer(binary):
    binary = binary.resolve(strict=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    calls = []
    with tempfile.TemporaryDirectory(prefix="myelin-remote-scarce-") as tmp:
        tmp = Path(tmp)

        def run(root, op, *files, accepted=True, lose_reply=False, crash=None):
            command = [str(binary), op, str(root)] + [str(file) for file in files]
            env = dict(os.environ)
            env.pop("MYELIN_AUTHORITY_CRASH_POINT", None)
            if crash:
                env["MYELIN_AUTHORITY_CRASH_POINT"] = crash
            result = subprocess.run(command, capture_output=True, text=True, check=False, timeout=30, env=env)
            call = {"arguments": command[1:], "exit_code": result.returncode, "stderr": result.stderr,
                    "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(),
                    "reply_lost_before_client_observation": lose_reply, "crash_point": crash}
            calls.append(call)
            expected = 74 if crash else (0 if accepted else 1)
            assert result.returncode == expected, (command, expected, result.returncode, result.stderr)
            if lose_reply:
                call["lost_response_for_audit"] = result.stdout
                return None
            value = json.loads(result.stdout) if result.returncode == 0 else None
            call["response"] = value
            return value

        def start(name):
            root = tmp / name
            initialized = run(root, "init")
            assert initialized["test_assets"] and initialized["different_authorities"]
            issued = run(root, "issue")
            assert run(root, "issue") == issued
            proof = tmp / (name + ".proof")
            run(root, "export", proof)
            run(root, "import", proof)
            return root, proof

        root, proof = start("consume")
        candidate, certificate, record = (tmp / name for name in ["consume.candidate", "consume.cert", "consume.record"])
        run(root, "prepare", proof, candidate, crash="after-remote-reservation")
        assert not candidate.exists()
        assert run(root, "target-status")["reserved"]
        prepared = run(root, "prepare", proof, candidate)
        assert prepared["target_height"] is None
        # Coordinator outage leaves the original pending candidate and source lock.
        root.joinpath("terminal").rename(root / "terminal-offline")
        run(root, "authorize", proof, candidate, certificate, accepted=False)
        run(root, "reclaim", proof, accepted=False)
        locked = run(root, "source-status")
        assert (locked["available"], locked["locked"]) == (75, 25)
        root.joinpath("terminal-offline").rename(root / "terminal")
        run(root, "authorize", proof, candidate, certificate, crash="after-terminal-decision")
        assert not certificate.exists()
        approved = run(root, "authorize", proof, candidate, certificate)
        assert approved["decision"] == "consume-authorized"
        # Source service is offline; only target/coordinator processes can proceed.
        root.joinpath("source").rename(root / "source-offline")
        run(root, "commit", certificate, crash="after-remote-commit")
        status = run(root, "target-status")
        assert status["height"] == 0 and not status["reserved"]
        run(root, "commit", certificate, lose_reply=True)
        lost = calls[-1]["lost_response_for_audit"]
        consumed = run(root, "commit", certificate)
        assert consumed == json.loads(lost)
        assert run(root, "record", record)["available"] == 25
        assert run(root, "authorize", proof, candidate, certificate) == approved
        run(root, "decide-cancel", proof, tmp / "forbidden.cert", accepted=False)
        root.joinpath("source-offline").rename(root / "source")
        run(root, "cancel", proof, certificate, accepted=False)
        run(root, "reclaim", proof, accepted=False)
        original = record.read_bytes()
        changed = bytearray(original)
        changed[-1] ^= 1
        record.write_bytes(changed)
        run(root, "complete", proof, record, accepted=False)
        record.write_bytes(original)
        run(root, "complete", proof, record, lose_reply=True)
        lost = calls[-1]["lost_response_for_audit"]
        completed = run(root, "complete", proof, record)
        assert completed == json.loads(lost)
        assert (completed["available"], completed["locked"], completed["pending"]) == (75, 0, 0)
        assert run(root, "replay-source")["state_root"] == completed["state_root"]
        root.joinpath("source").rename(root / "source-retired-offline")
        target_replay = run(root, "replay-target")
        assert target_replay["state_root"] == bytes(consumed["state_root"]).hex()
        assert not run(root, "import", proof)["available"]

        cancelled_root, cancelled_proof = start("cancel")
        pending, cancelled = tmp / "cancel.candidate", tmp / "cancel.cert"
        run(cancelled_root, "prepare", cancelled_proof, pending)
        run(cancelled_root, "decide-cancel", cancelled_proof, cancelled, crash="after-terminal-decision")
        decision = run(cancelled_root, "decide-cancel", cancelled_proof, cancelled)
        assert decision["decision"] == "cancelled"
        run(cancelled_root, "commit", cancelled, accepted=False)
        run(cancelled_root, "release", cancelled)
        run(cancelled_root, "cancel", cancelled_proof, cancelled, lose_reply=True)
        lost = calls[-1]["lost_response_for_audit"]
        assert run(cancelled_root, "cancel", cancelled_proof, cancelled) == json.loads(lost)
        run(cancelled_root, "reclaim", cancelled_proof, lose_reply=True)
        lost = calls[-1]["lost_response_for_audit"]
        refunded = run(cancelled_root, "reclaim", cancelled_proof)
        assert refunded == json.loads(lost)
        assert (refunded["available"], refunded["locked"]) == (100, 0)
        # Reordered relay and a delayed candidate with the original old timestamp.
        run(cancelled_root, "import", cancelled_proof)
        run(cancelled_root, "prepare", cancelled_proof, pending)
        run(cancelled_root, "authorize", cancelled_proof, pending, tmp / "too-late.cert", accepted=False)
        run(cancelled_root, "release", cancelled)
        assert run(cancelled_root, "target-status")["height"] is None
        assert run(cancelled_root, "replay-source")["state_root"] == refunded["state_root"]

        cloned_root, cloned_proof = start("clones")
        clone = tmp / "second-target"
        clone.mkdir()
        for filename in ["source.config", "target.config"]:
            shutil.copyfile(cloned_root / filename, clone / filename)
        # No live handles: copy the exact target's canonical/ownership pair.
        shutil.copytree(cloned_root / "target", clone / "target")
        a, b, selected = tmp / "a.candidate", tmp / "b.candidate", tmp / "clone.cert"
        run(cloned_root, "prepare", cloned_proof, a, 1)
        run(clone, "prepare", cloned_proof, b, 2)
        assert a.read_bytes() != b.read_bytes()
        run(cloned_root, "authorize", cloned_proof, a, selected)
        run(cloned_root, "authorize", cloned_proof, b, tmp / "second.cert", accepted=False)
        run(clone, "commit", selected, accepted=False)
        run(cloned_root, "commit", selected)
        run(cloned_root, "decide-cancel", cloned_proof, tmp / "clone-cancel.cert", accepted=False)
        run(cloned_root, "reclaim", cloned_proof, accepted=False)
        assert run(clone, "target-status")["height"] is None
        assert run(cloned_root, "record", tmp / "clone.record")["available"] == 25
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    return {"binary": str(binary), "binary_sha256": digest, "calls": calls, "passed": True,
            "scope": "scarce test assets; independent native processes/stores/PoA authorities; one trusted durable non-rollback terminal authority; file relay; no L1/real custody",
            "consumed": consumed, "completed": completed, "refunded": refunded, "target_replay": target_replay}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--remote-binary", type=Path, required=True)
    parser.add_argument("--terminal-binary", type=Path, required=True)
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
    remote = remote_receipt(args.remote_binary)
    terminal = terminal_transfer(args.terminal_binary)
    local_calls = sum(len(c["calls"]) for c in cases)
    args.output.write_text(json.dumps({"binary": str(binary), "binary_sha256": binary_hash,
                                      "cases": cases, "remote_receipt": remote, "remote_scarce": terminal, "passed": True}, indent=2) + "\n")
    print(json.dumps({"passed": True, "target_modes": len(cases), "shared_store_invocations": local_calls,
                      "remote_process_invocations": len(remote["calls"]), "scarce_process_invocations": len(terminal["calls"]),
                      "process_invocations": local_calls + len(remote["calls"]) + len(terminal["calls"])}))


if __name__ == "__main__":
    main()
