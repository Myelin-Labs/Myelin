#!/usr/bin/env python3
"""
Generate the local, uncommitted reports/myelin-teeworlds-repro.json from the
live Teeworlds acceptance output. Run after
`bash scripts/myelin_teeworlds_acceptance.sh` to merge the static-committee and
weighted-precommit reports into a single reproducible JSON artefact.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

MYELIN_ROOT = Path(__file__).resolve().parent.parent
# TEEWORLDS_ROOT is overridable. The default is the local-machine path
# used during the standalone Myelin audit; CI / other developers should
# set TEEWORLDS_ROOT to their own clone.
TEEWORLDS_ROOT = os.environ.get("TEEWORLDS_ROOT", str(Path.home() / "RustroverProjects" / "teeworlds"))
# ACCEPTANCE_DIR is the directory the Teeworlds acceptance script writes
# its raw reports into. The acceptance script defaults to
# /tmp/myelin-teeworlds-acceptance; if it was redirected, ACCEPTANCE_DIR
# must agree.
ACCEPTANCE_DIR = os.environ.get("MYELIN_TEEWORLDS_ACCEPTANCE_DIR", "/tmp/myelin-teeworlds-acceptance")
REPORT_PATH = MYELIN_ROOT / "reports" / "myelin-teeworlds-repro.json"

def run_myelin(*args):
    return subprocess.run(
        ["cargo", "run", "-q", "-p", "myelin-cli", "--", *args],
        cwd=MYELIN_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )

def load_or_run_static(mock_tx_path):
    """Run inspect / bench with static-closed-committee and load the reports."""
    build_dir = Path(ACCEPTANCE_DIR)
    build = json.loads((build_dir / "build-fixture.json").read_text())
    vm = json.loads((build_dir / "vm-probe.json").read_text())
    court = json.loads((build_dir / "court-bundle.json").read_text())
    verify = json.loads((build_dir / "court-bundle-verify.json").read_text())
    return build, vm, court, verify

def run_weighted_precommit(mock_tx_path):
    """Run inspect / court-bundle with --consensus weighted-precommit."""
    out_dir = Path("/tmp/myelin-weighted-precommit-teeworlds")
    out_dir.mkdir(parents=True, exist_ok=True)
    inspect_path = out_dir / "inspect-weighted-precommit.json"
    court_path = out_dir / "court-bundle-weighted-precommit.json"
    verify_path = out_dir / "court-bundle-verify-weighted-precommit.json"
    run_myelin("teeworlds", "inspect", "--mock-tx", mock_tx_path, "--consensus", "weighted-precommit", "--out", str(inspect_path))
    run_myelin("teeworlds", "court-bundle", "--mock-tx", mock_tx_path, "--consensus", "weighted-precommit", "--out", str(court_path))
    run_myelin("teeworlds", "verify-court-bundle", "--bundle", str(court_path), "--out", str(verify_path))
    return json.loads(inspect_path.read_text()), json.loads(court_path.read_text()), json.loads(verify_path.read_text())

def main():
    mock_tx = Path(ACCEPTANCE_DIR) / "teeworlds-mock-tx.json"
    if not mock_tx.exists():
        print(f"missing {mock_tx}; run scripts/myelin_teeworlds_acceptance.sh first", file=sys.stderr)
        sys.exit(1)

    build, vm, court, verify = load_or_run_static(str(mock_tx))
    wp_inspect, wp_court, wp_verify = run_weighted_precommit(str(mock_tx))

    fixture = build["benchmark"]["fixture"]
    chunks = fixture["chunks"]

    report = {
        "schema": "myelin-teeworlds-repro-v2",
        "teeworlds_root": TEEWORLDS_ROOT,
        "static_committee": {
            "fixture": {
                "tape_bytes": fixture["tape_bytes"],
                "chunks": len(chunks),
                "chunk_bytes": fixture["chunk_bytes"],
                "wire_encoded": fixture["wire_encoded"],
                "projection_stage": chunks[0]["ckb_projection"]["projection_stage"],
                "average_elapsed_ns": build["benchmark"]["average_elapsed_ns"],
                "finalised": fixture["finality"]["finalised"],
                "block_hash": fixture["finality"]["block_hash"],
                "consensus_kind": fixture["finality"]["consensus_kind"],
                "signer_ids": fixture["finality"]["signer_ids"],
                "quorum_weight": fixture["finality"]["quorum_weight"],
            },
            "vm_probe": {
                "success": vm["success"],
                "ckb_strict": vm["ckb_strict"],
                "cycles": vm["cycles"],
                "max_cycles": vm["max_cycles"],
                "replayer": vm["replayer"],
            },
            "court_bundle": {
                "court_verifiable": court["court_verifiable"],
                "l1_court_implemented": court["l1_court_implemented"],
                "molecule_transaction_bytes": court["molecule_transaction_bytes"],
                "static_committee_signatures": len(court["static_committee_evidence"]["signatures"]),
                "static_committee_quorum_weight": court["static_committee_evidence"]["quorum_weight"],
                "static_committee_finalised": court["static_committee_evidence"]["finalised"],
            },
            "court_bundle_verification": {
                "valid": verify["valid"],
                "checks": len(verify["checks"]),
                "failed_checks": [c["name"] for c in verify["checks"] if not c["ok"]],
            },
        },
        "weighted_precommit": {
            "fixture": {
                "tape_bytes": wp_inspect["tape_bytes"],
                "chunks": len(wp_inspect["chunks"]),
                "wire_encoded": wp_inspect["wire_encoded"],
                "projection_stage": wp_inspect["chunks"][0]["ckb_projection"]["projection_stage"],
                "block_hash": wp_inspect["finality"]["block_hash"],
                "consensus_kind": wp_inspect["finality"]["consensus_kind"],
                "certificate_height": wp_inspect["finality"]["certificate_height"],
                "certificate_round": wp_inspect["finality"]["certificate_round"],
                "certificate_step": wp_inspect["finality"]["certificate_step"],
                "signer_ids": wp_inspect["finality"]["signer_ids"],
                "quorum_weight": wp_inspect["finality"]["quorum_weight"],
                "finalised": wp_inspect["finality"]["finalised"],
            },
            "court_bundle": {
                "court_verifiable": wp_court["court_verifiable"],
                "l1_court_implemented": wp_court["l1_court_implemented"],
                "molecule_transaction_bytes": wp_court["molecule_transaction_bytes"],
                "weighted_precommit_evidence": wp_court["weighted_precommit_evidence"],
            },
            "court_bundle_verification": {
                "valid": wp_verify["valid"],
                "checks": len(wp_verify["checks"]),
                "failed_checks": [c["name"] for c in wp_verify["checks"] if not c["ok"]],
            },
        },
        "shared_metrics": {
            "tape_bytes": vm["tape_bytes"],
            "vm_cycles": vm["cycles"],
            "projection_stage": "wire-encoded",
            "court_bundle_status": "internally-valid-not-ckb-context-verified",
        },
    }

    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(json.dumps(report, indent=2, sort_keys=True))
    print(f"wrote {REPORT_PATH}")
    print(json.dumps(report["shared_metrics"], indent=2, sort_keys=True))

if __name__ == "__main__":
    main()
