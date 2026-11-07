#!/usr/bin/env python3
"""Package the court drill, or verify independently retrieved pinned evidence.

The manifest digest is an external trust input. Local copies do not establish
independent DA; the retrieval receipt names the actual provider and artifact.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tomllib
from myelin_microgame import (N, SCHEMA, VM, cell_output, da_preimage, h, hx, point, raw, raw_tx, transaction, u32, write_json)

MAX_BYTES = 256 * 1024 * 1024


def digest(path):
    assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 32 * 1024 * 1024, "artifact exceeds individual byte ceiling or is not regular"
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(args):
    root = args.evidence.resolve(strict=True)
    assert root.is_dir() and not (root / "manifest.json").exists()
    fixture = Path(__file__).resolve().parents[1] / "fixtures/ckb-microgame"
    from validation import source_identity
    write_json(root / "source.json", source_identity(Path(__file__).resolve().parents[1]))
    shutil.copyfile(args.replayer, root / "replayer")
    (root / "replayer").chmod(0o755)
    build = root / "build"
    build.mkdir()
    for name in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "devnet.toml", "always_success", "NOTICE.md", "COPYING.Nervos", "contract/Cargo.toml", "contract/src/main.rs", "runner/Cargo.toml", "runner/src/main.rs"]:
        destination = build / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(fixture / name, destination)
    write_json(build / "compiler.json", {"rustc": subprocess.check_output(["rustc", "+1.95.0", "-vV"], text=True),
                                        "clang": subprocess.check_output(["clang", "--version"], text=True),
                                        "target": "riscv64imac-unknown-none-elf", "target_rustflags": "-C target-feature=-a -C passes=lower-atomic",
                                        "replayer_sha256": digest(args.replayer), "program_sha256": digest(root / "program")})
    files = {}
    total = 0
    for path in sorted(root.rglob("*")):
        assert not path.is_symlink()
        if path.is_file():
            size = path.stat().st_size
            total += size
            assert total <= MAX_BYTES
            files[str(path.relative_to(root))] = {"bytes": size, "sha256": digest(path)}
    manifest = {"format": "myelin-ckb-microgame-archive-v1", "files": files, "total_bytes": total}
    write_json(root / "manifest.json", manifest)
    return {"manifest_sha256": digest(root / "manifest.json"), "bytes": total, "files": len(files)}


def verify(args):
    root = args.evidence.resolve(strict=True)
    assert (root / "manifest.json").stat().st_size <= 1024 * 1024, "manifest exceeds byte ceiling"
    assert digest(root / "manifest.json") == args.manifest_sha256, "external manifest identity differs"
    manifest = json.loads((root / "manifest.json").read_text())
    assert manifest["format"] == "myelin-ckb-microgame-archive-v1" and 0 < len(manifest["files"]) <= 2048
    total = 0
    for name, identity in manifest["files"].items():
        relative = PurePosixPath(name)
        assert not relative.is_absolute() and ".." not in relative.parts and str(relative) == name
        path = root / name
        assert not path.is_symlink() and path.is_file() and root in path.resolve().parents
        assert isinstance(identity["bytes"], int) and 0 <= identity["bytes"] <= 32 * 1024 * 1024
        total += identity["bytes"]
        assert total <= MAX_BYTES and path.stat().st_size == identity["bytes"] and digest(path) == identity["sha256"], name
    assert total == manifest["total_bytes"]
    actual_files = {str(path.relative_to(root)) for path in root.rglob("*") if path.is_file()}
    assert actual_files == set(manifest["files"]) | {"manifest.json"}
    report = json.loads((root / "report.json").read_text())
    deployment = json.loads((root / "deployment.json").read_text())
    program, genesis = report["program_hash"], report["genesis"]
    assert report["passed"] and hx(h((root / "program").read_bytes())) == program == deployment["program_hash"]
    assert tomllib.loads((root / "build/devnet.toml").read_text())["params"]["max_block_cycles"] == 10_000_000
    assert deployment["max_block_cycles"] == 10_000_000 and digest(root / "build/devnet.toml") == deployment["chain_spec_sha256"]
    assert deployment["genesis"] == genesis and deployment["schema_hash"] == hx(h(SCHEMA)) and deployment["vm_profile_hash"] == hx(h(VM))
    compiler = json.loads((root / "build/compiler.json").read_text())
    assert digest(args.replayer) == compiler["replayer_sha256"], "use the independently pinned original replayer"
    blocks, cells, included = {}, {}, {}
    for path in sorted((root / "blocks").glob("*.json")):
        block_hash = "0x" + path.stem
        value = subprocess.run([str(args.replayer), "verify-block", str(path), block_hash], capture_output=True, text=True, check=True)
        checked = json.loads(value.stdout)
        block = json.loads(path.read_text())
        blocks[block_hash] = checked
        for tx in block["transactions"]:
            tx_hash = hx(h(raw_tx(tx)))
            assert tx_hash == tx["hash"] and tx_hash in checked["transactions"]
            included[tx_hash] = tx
            for index, output_ in enumerate(tx["outputs"]):
                key = raw(tx_hash) + u32(index)
                cell = (cell_output(output_), raw(tx["outputs_data"][index]))
                assert key not in cells or cells[key] == cell
                cells[key] = cell
    headers = json.loads((root / "canonical-headers.json").read_text())
    assert headers[0]["hash"] == genesis and headers[-1]["hash"] == report["canonical_tip"]
    assert len(headers) == report["canonical_height"] + 1
    canonical = {}
    live = {}
    for height, header in enumerate(headers):
        assert int(header["number"], 16) == height and (height == 0 or header["parent_hash"] == headers[height - 1]["hash"])
        checked = blocks[header["hash"]]
        assert checked["number"] == height and checked["parent_hash"] == header["parent_hash"]
        canonical[height] = header["hash"]
        # Reconstruct one canonical single-spend domain. Full script replay below
        # is separate from this retained-history/receipt closure check.
        block = json.loads((root / "blocks" / (header["hash"][2:] + ".json")).read_text())
        for index, tx in enumerate(block["transactions"]):
            if height > 0 and index > 0:
                for input_ in tx["inputs"]:
                    key = point(input_["previous_output"])
                    assert key in live, "canonical input is missing or spent twice"
                    del live[key]
            tx_hash = hx(h(raw_tx(tx)))
            for output_index, output_ in enumerate(tx["outputs"]):
                live[raw(tx_hash) + u32(output_index)] = (cell_output(output_), raw(tx["outputs_data"][output_index]))
    for receipt in report["receipts"]:
        tx = json.loads((root / "transactions" / (receipt["label"] + ".json")).read_text())
        tx_hash = hx(h(raw_tx(tx)))
        assert tx_hash == receipt["tx_hash"] == receipt["accepted"]["rpc_returned_hash"]
        assert transaction(tx) == transaction(included[tx_hash]), "witnesses differ from included transaction"
        committed, confirmed = receipt["committed"], receipt["confirmed"]
        block = blocks[committed["block_hash"]]
        assert block["number"] == committed["height"] and tx_hash in block["transactions"]
        assert confirmed["minimum_depth"] == report["configured_depth"] and confirmed["observed_depth"] == confirmed["tip_height"] - committed["height"] + 1
        assert confirmed["observed_depth"] >= confirmed["minimum_depth"]
        cursor = blocks[confirmed["tip_hash"]]
        assert cursor["number"] == confirmed["tip_height"]
        while cursor["number"] > committed["height"]:
            cursor = blocks[cursor["parent_hash"]]
        assert cursor["block_hash"] == committed["block_hash"], "confirmation tip is not a descendant"
        current = canonical.get(committed["height"]) == committed["block_hash"]
        assert current == ("after_reorg" not in receipt), "reorg status differs from selected canonical history"
    proof_count = 0
    for check in report["checks"]:
        if "proof" not in check:
            continue
        path = root / check["proof"]
        source = json.loads(path.read_text())
        assert source["program_hash"] == program and source["genesis"] == genesis
        for cell in source["cells"]:
            assert cells[raw(cell["out_point"])] == (raw(cell["output"]), raw(cell["data"])), "resolved state/dependency lacks retained transaction inclusion"
        result = subprocess.run([str(args.replayer), "verify", str(path), program, genesis], capture_output=True, text=True, check=False, timeout=30)
        verdict = json.loads(result.stdout)
        assert result.returncode == (0 if check["expected_valid"] else 1) and verdict["valid"] == check["expected_valid"], (check["label"], verdict)
        assert verdict["tx_hash"] == check["result"]["tx_hash"]
        proof_count += 1
    for move in report["moves"]:
        preimage = (root / move["input"]).read_bytes()
        assert hx(h(preimage)) == move["root"]
        tx = included[move["proposal_tx"]]
        state = raw(tx["outputs_data"][0])
        assert len(state) == N and da_preimage(state) == preimage and hx(state[289:321]) == move["root"]
        assert hx(state[225:257]) == move["pre_root"] and hx(state[257:289]) == move["claimed_post_root"]
    return {"format": "myelin-ckb-microgame-retrieval-v1", "passed": True, "manifest_sha256": args.manifest_sha256,
            "provider": args.provider, "retrieved_external_artifact": args.artifact_id, "genesis": genesis, "program_hash": program,
            "proofs_reexecuted": proof_count, "blocks_rehashed": len(blocks), "receipts_reverified": len(report["receipts"]),
            "da_inputs_retrieved": len(report["moves"]), "canonical_tip": report["canonical_tip"],
            "scope": "pinned archived test-asset evidence; trusted Dummy-PoW devnet tip; provider availability at retrieval only; no current public-chain finality or indefinite DA claim"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["package", "verify"])
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replayer", type=Path, required=True)
    parser.add_argument("--manifest-sha256")
    parser.add_argument("--provider")
    parser.add_argument("--artifact-id")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    args.replayer = args.replayer.resolve(strict=True)
    if args.operation == "verify":
        assert args.manifest_sha256 and args.provider and args.artifact_id
    result = package(args) if args.operation == "package" else verify(args)
    if args.out:
        write_json(args.out, result)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
