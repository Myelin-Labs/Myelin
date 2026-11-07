#!/usr/bin/env python3
"""Run the strict microgame's actual CKB devnet transactions and retain evidence."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request
from myelin_microgame import (ALWAYS, C, FEE, KEYS, SCHEMA, STAKE, TIMEOUT, VM, cell_input, da_preimage, fraud, h, hx, initial, moved, new_tx, output, point, proof, propose, public, raw, raw_tx, root, script, sign_tx, u64, write_json)


class RPCError(RuntimeError):
    def __init__(self, error):
        self.error = error
        super().__init__(json.dumps(error, sort_keys=True))


class Drill:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.out.mkdir(parents=True, exist_ok=False)
        self.evidence = self.out / "evidence"
        self.evidence.mkdir()
        for name in ["transactions", "proofs", "blocks", "inputs"]:
            (self.evidence / name).mkdir()
        self.chain = self.out / "chain"
        self.chain.mkdir()
        self.rpc_url = None
        self.cells = {}
        self.receipts = []
        self.checks = []
        self.moves = []
        self.wallets = []
        self.dependencies = []
        self.runner = args.replayer.resolve(strict=True)
        self.program = args.program.resolve(strict=True).read_bytes()
        self.program_hash = hx(h(self.program))
        self.genesis = None

    def rpc(self, method, params=None):
        body = json.dumps({"id": 1, "jsonrpc": "2.0", "method": method, "params": params or []}).encode()
        request = urllib.request.Request(self.rpc_url, data=body, headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=30) as response:
            value = json.load(response)
        if "error" in value:
            raise RPCError(value["error"])
        return value["result"]

    def mine(self, count=1):
        for _ in range(count):
            height = int(self.rpc("get_tip_header")["number"], 16)
            for _ in range(300):
                self.rpc("generate_block")
                if int(self.rpc("get_tip_header")["number"], 16) > height:
                    break
                time.sleep(.01)
            else:
                raise RuntimeError("generated block did not become canonical")

    def cell(self, tx, index):
        out_point = {"tx_hash": hx(h(raw_tx(tx))), "index": hex(index)}
        return {"out_point": out_point, "output": tx["outputs"][index], "data": tx["outputs_data"][index]}

    def remember(self, tx):
        for index in range(len(tx["outputs"])):
            item = self.cell(tx, index)
            self.cells[point(item["out_point"])] = item

    def save_block(self, block_hash):
        path = self.evidence / "blocks" / (block_hash[2:] + ".json")
        if not path.exists():
            write_json(path, self.rpc("get_block", [block_hash]))
        verified = subprocess.run([str(self.runner), "verify-block", str(path), block_hash], capture_output=True, text=True, check=True)
        return json.loads(verified.stdout)

    def vm(self, label, tx, inputs, valid=True):
        cells = {point(item["out_point"]): item for item in inputs + self.dependencies}
        path = self.evidence / "proofs" / (label + ".json")
        write_json(path, proof(tx, list(cells.values()), self.program_hash, self.genesis))
        result = subprocess.run([str(self.runner), "verify", str(path), self.program_hash, self.genesis], capture_output=True, text=True, timeout=30, check=False)
        value = json.loads(result.stdout)
        assert result.returncode == (0 if valid else 1), (label, result.returncode, value, result.stderr)
        assert value["valid"] == valid, (label, value)
        self.checks.append({"label": label, "expected_valid": valid, "proof": str(path.relative_to(self.evidence)), "result": value})
        return value

    def submit(self, label, tx, inputs=None, valid=True, vm=True):
        tx_hash = hx(h(raw_tx(tx)))
        write_json(self.evidence / "transactions" / (label + ".json"), tx)
        if vm:
            self.vm(label, tx, inputs or [], valid)
        if not valid:
            try:
                self.rpc("send_transaction", [tx, "passthrough"])
            except RPCError as error:
                assert "script" in str(error).lower() or "validationfailure" in str(error).lower(), (label, str(error))
                self.checks[-1]["node_rejection"] = error.error
                return None
            raise AssertionError((label, "incorrect script was accepted by CKB"))
        accepted = self.rpc("send_transaction", [tx, "passthrough"])
        assert accepted == tx_hash
        pending = self.rpc("get_transaction", [tx_hash])
        receipt = {"label": label, "tx_hash": tx_hash, "accepted": {"rpc_returned_hash": accepted, "status": pending["tx_status"]["status"]}, "committed": None, "confirmed": None}
        self.receipts.append(receipt)
        for _ in range(20):
            self.mine()
            observed = self.rpc("get_transaction", [tx_hash])
            if observed["tx_status"]["status"] == "committed":
                break
        else:
            raise AssertionError((label, "did not commit", observed))
        block_hash = observed["tx_status"]["block_hash"]
        block = self.save_block(block_hash)
        assert tx_hash in block["transactions"]
        assert self.rpc("get_block_hash", [hex(block["number"])]) == block_hash
        receipt["committed"] = {"block_hash": block_hash, "height": block["number"], "locally_reverified": True}
        while int(self.rpc("get_tip_header")["number"], 16) - block["number"] + 1 < self.args.depth:
            self.mine()
        tip = self.rpc("get_tip_header")
        for height in range(block["number"], int(tip["number"], 16) + 1):
            self.save_block(self.rpc("get_block_hash", [hex(height)]))
        receipt["confirmed"] = {"minimum_depth": self.args.depth, "observed_depth": int(tip["number"], 16) - block["number"] + 1, "tip_hash": tip["hash"], "tip_height": int(tip["number"], 16), "canonical_at_observation": True}
        self.remember(tx)
        return receipt

    def setup(self):
        fixture = Path(__file__).resolve().parents[1] / "fixtures/ckb-microgame"
        always = (fixture / "always_success").read_bytes()
        assert hx(h(always)) == ALWAYS
        spec = (fixture / "devnet.toml").read_text().replace('file = { file = "always_success" }', 'file = { file = "' + str((fixture / "always_success").resolve()) + '" }')
        spec_path = self.out / "devnet.toml"
        spec_path.write_text(spec)
        # Hold both sockets while selecting distinct unused local ports.
        with socket.socket() as rpc_socket, socket.socket() as p2p_socket:
            rpc_socket.bind(("127.0.0.1", 0))
            p2p_socket.bind(("127.0.0.1", 0))
            ports = [rpc_socket.getsockname()[1], p2p_socket.getsockname()[1]]
        self.rpc_url = f"http://127.0.0.1:{ports[0]}"
        with (self.out / "init.log").open("w") as log:
            subprocess.run([str(self.args.ckb), "-C", str(self.chain), "init", "--chain", "dev", "--import-spec", str(spec_path), "--rpc-port", str(ports[0]), "--p2p-port", str(ports[1]), "--ba-code-hash", ALWAYS, "--ba-hash-type", "data", "--ba-arg", "0x", "--force", "--log-to", "stdout"], stdout=log, stderr=subprocess.STDOUT, check=True)
        config = self.chain / "ckb.toml"
        value = config.read_text()
        # This isolated Dummy-PoW fixture explicitly enables test RPCs. It never
        # accepts a public network or reuses an operator's node directory.
        value = value.replace('"Debug"', '"Debug", "IntegrationTest"').replace("/ip4/0.0.0.0/", "/ip4/127.0.0.1/")
        config.write_text(value)
        self.log = (self.out / "node.log").open("w")
        self.node = subprocess.Popen([str(self.args.ckb), "-C", str(self.chain), "run", "--ba-advanced"], stdout=self.log, stderr=subprocess.STDOUT)
        for _ in range(300):
            if self.node.poll() is not None:
                raise RuntimeError("CKB exited: " + (self.out / "node.log").read_text()[-3000:])
            try:
                self.rpc("get_tip_header")
                break
            except (urllib.error.URLError, OSError):
                time.sleep(.1)
        else:
            raise RuntimeError("CKB RPC not ready")
        self.genesis = self.rpc("get_block_hash", ["0x0"])
        genesis = self.rpc("get_block_by_number", ["0x0"])
        self.save_block(self.genesis)
        for tx in genesis["transactions"]:
            self.remember(tx)
        system = genesis["transactions"][0]
        always_index = next(i for i, data in enumerate(system["outputs_data"]) if hx(h(raw(data))) == ALWAYS)
        self.always_lock = {"code_hash": ALWAYS, "hash_type": "data", "args": "0x"}
        secp_code = system["outputs"][1]["type"]
        self.player_locks = [{"code_hash": hx(h(script(secp_code))), "hash_type": "type", "args": hx(h(public(key))[:20])} for key in KEYS]
        group = self.cell(genesis["transactions"][1], 0)
        self.base_deps = [{"out_point": self.cell(system, always_index)["out_point"], "dep_type": "code"}, {"out_point": group["out_point"], "dep_type": "dep_group"}]
        # Resolved children of the real genesis secp dep-group, plus the group.
        self.dependencies = [self.cell(system, always_index), group, self.cell(system, 1), self.cell(system, 3)]
        self.mine(96)
        code_capacity = (len(self.program) + 42) * C
        wallet = 20_000 * C
        funding, capacity = [], 0
        for height in range(1, 97):
            block = self.rpc("get_block_by_number", [hex(height)])
            funding_tx = block["transactions"][0]
            if funding_tx["outputs"]:
                self.remember(funding_tx)
                item = self.cell(funding_tx, 0)
                funding.append(item)
                capacity += int(item["output"]["capacity"], 16)
                if capacity >= code_capacity + wallet * 2 + FEE + 100 * C:
                    break
        assert capacity >= code_capacity + wallet * 2 + FEE + 100 * C, "insufficient finalized test-asset rewards"
        tx = new_tx(funding, [output(code_capacity, self.always_lock), output(wallet, self.player_locks[0]), output(wallet, self.player_locks[1]), output(capacity - code_capacity - wallet * 2 - FEE, self.always_lock)], [self.program, b"", b"", b""], [self.base_deps[0]])
        self.submit("deploy-and-fund", tx, vm=False)
        self.code = self.cell(tx, 0)
        self.wallets = [self.cell(tx, 1), self.cell(tx, 2)]
        self.dependencies.append(self.code)
        self.deps = self.base_deps + [{"out_point": self.code["out_point"], "dep_type": "code"}]
        (self.evidence / "program").write_bytes(self.program)
        write_json(self.evidence / "deployment.json", {"genesis": self.genesis, "program_hash": self.program_hash, "program_sha256": hashlib.sha256(self.program).hexdigest(), "schema_hash": hx(h(SCHEMA)), "vm_profile_hash": hx(h(VM)), "players": self.player_locks, "stake_shannons": STAKE, "timeout_relative_blocks": TIMEOUT, "fee_shannons": FEE, "script_hash_type": "data2", "code_dep": self.code["out_point"], "max_block_cycles": 10_000_000, "chain_spec_sha256": hashlib.sha256((fixture / "devnet.toml").read_bytes()).hexdigest(), "ckb_binary_sha256": hashlib.sha256(self.args.ckb.read_bytes()).hexdigest(), "ckb_version": subprocess.check_output([str(self.args.ckb), "--version"], text=True).strip()})

    def open_game(self, label):
        inputs = self.wallets.copy()
        game_id = h(cell_input({"since": "0x0", "previous_output": inputs[0]["out_point"]}) + u64(0))
        type_ = {"code_hash": self.program_hash, "hash_type": "data2", "args": hx(game_id)}
        state = initial(self.program_hash, game_id, self.player_locks)
        outputs = [output(STAKE * 2, self.always_lock, type_)] + [output(int(item["output"]["capacity"], 16) - STAKE - (FEE if i == 0 else 0), self.player_locks[i]) for i, item in enumerate(inputs)]
        tx = sign_tx(new_tx(inputs, outputs, [state, b"", b""], self.deps), inputs, self.player_locks)
        self.submit(label, tx, inputs)
        self.wallets = [self.cell(tx, 1), self.cell(tx, 2)]
        return self.cell(tx, 0)

    def update(self, label, game, state, action, actor=0, valid=True):
        inputs = [game, self.wallets[actor]]
        tx = new_tx(inputs, [copy.deepcopy(game["output"]), output(int(inputs[1]["output"]["capacity"], 16) - FEE, self.player_locks[actor])], [state, b""], self.deps, action)
        sign_tx(tx, inputs, self.player_locks)
        receipt = self.submit(label, tx, inputs, valid)
        if valid:
            self.wallets[actor] = self.cell(tx, 1)
            if action == 1:
                preimage = da_preimage(state)
                assert h(preimage) == state[289:321]
                path = self.evidence / "inputs" / (label + ".bin")
                path.write_bytes(preimage)
                self.moves.append({"input": str(path.relative_to(self.evidence)), "root": hx(h(preimage)), "proposal_tx": receipt["tx_hash"], "claimed_post_root": hx(state[257:289]), "pre_root": hx(state[225:257])})
            return self.cell(tx, 0)
        return game

    def exit_tx(self, game, action, winner=None, since=0, actor=0):
        inputs = [game, self.wallets[actor]]
        outputs = [output(STAKE * 2, self.player_locks[winner])] if winner is not None else [output(STAKE, lock) for lock in self.player_locks]
        outputs.append(output(int(inputs[1]["output"]["capacity"], 16) - FEE, self.player_locks[actor]))
        tx = new_tx(inputs, outputs, [b""] * len(outputs), self.deps, action, since)
        return sign_tx(tx, inputs, self.player_locks), inputs

    def exit(self, label, game, action, winner=None, since=0, valid=True, actor=0):
        tx, inputs = self.exit_tx(game, action, winner, since, actor)
        result = self.submit(label, tx, inputs, valid)
        if valid:
            self.wallets[actor] = self.cell(tx, len(tx["outputs"]) - 1)
        return tx, result

    def run(self):
        self.setup()
        normal = self.open_game("normal-open")
        start = raw(normal["data"])
        proposed = propose(start, 2)
        wrong_profile = bytearray(proposed)
        wrong_profile[68] ^= 1
        self.update("wrong-vm-profile", normal, bytes(wrong_profile), 1, valid=False)
        inputs = [normal, self.wallets[0]]
        bad_auth = new_tx(inputs, [copy.deepcopy(normal["output"]), output(int(inputs[1]["output"]["capacity"], 16) - FEE, self.player_locks[0])], [proposed, b""], self.deps, 1)
        sign_tx(bad_auth, inputs, self.player_locks)
        signed_witness = bytearray(raw(bad_auth["witnesses"][1]))
        signed_witness[20] ^= 1
        bad_auth["witnesses"][1] = hx(signed_witness)
        self.submit("forged-participant-signature", bad_auth, inputs, valid=False)
        self.update("unauthorized-move", normal, proposed, 1, actor=1, valid=False)
        normal = self.update("normal-commit-1", normal, proposed, 1)
        wrong = bytearray(moved(proposed))
        wrong[221] = 3
        wrong[225:257] = root(wrong)
        self.update("incorrect-game-result", normal, bytes(wrong), 2, valid=False)
        self.update("false-challenge", normal, fraud(proposed), 3, actor=1, valid=False)
        normal = self.update("normal-finalize-1", normal, moved(proposed), 2)
        proposed = propose(raw(normal["data"]), 2)
        normal = self.update("normal-commit-2", normal, proposed, 1, actor=1)
        normal = self.update("normal-finalize-2", normal, moved(proposed), 2)
        proposed = propose(raw(normal["data"]), 1)
        normal = self.update("normal-commit-3", normal, proposed, 1)
        normal = self.update("normal-finalize-3", normal, moved(proposed), 2)
        # A completed win cannot be changed into split refunds by recovery.
        self.mine(TIMEOUT + 1)
        self.exit("winner-cannot-refund", normal, 5, since=(1 << 63) | TIMEOUT, valid=False)
        before_settle = self.rpc("get_tip_header")
        wallet_before_settle = copy.deepcopy(self.wallets)
        settlement, receipt = self.exit("normal-settle", normal, 4, winner=0)
        # Exact stale outpoint and duplicate claim rejection are node/custody checks.
        for label, action in [("duplicate-normal-exit", 4), ("stale-timeout-exit", 5)]:
            stale, _ = self.exit_tx(normal, action, winner=0, since=(1 << 63) | TIMEOUT if action == 5 else 0)
            try:
                self.rpc("send_transaction", [stale, "passthrough"])
            except RPCError as error:
                assert normal["out_point"]["tx_hash"][2:] in str(error) and "Unknown" in str(error)
                self.checks.append({"label": label, "expected_valid": False, "node_rejection": error.error, "assurance": "spent canonical game Cell"})
            else:
                raise AssertionError(label)
        # Real devnet rollback: the former confirmation is revoked, then one
        # recovery exit spends the revived same custody Cell on the new history.
        self.rpc("truncate", [before_settle["hash"]])
        self.rpc("clear_tx_pool")
        old_height = receipt["committed"]["height"]
        canonical = self.rpc("get_block_hash", [hex(old_height)])
        assert canonical != receipt["committed"]["block_hash"]
        receipt["after_reorg"] = {"canonical": False, "current_hash_at_old_height": canonical, "status": "reorged"}
        self.wallets = wallet_before_settle
        self.mine(TIMEOUT + 1)
        self.exit("recovery-winner-timeout", normal, 5, winner=0, since=(1 << 63) | TIMEOUT)
        try:
            self.rpc("send_transaction", [settlement, "passthrough"])
        except RPCError as error:
            assert normal["out_point"]["tx_hash"][2:] in str(error) and "Unknown" in str(error)
            self.checks.append({"label": "normal-versus-recovery-single-spend", "expected_valid": False, "node_rejection": error.error})
        else:
            raise AssertionError("both normal/recovery exit accepted")
        # A genuinely incorrect pending result may be proposed, but cannot be
        # finalized. A successful challenge produces the opposite winner.
        for bad in ["counter", "proof"]:
            game = self.open_game("fraud-" + bad + "-open")
            state = propose(raw(game["data"]), 2, invalid_counter=bad == "counter", invalid_root=bad == "proof")
            game = self.update("fraud-" + bad + "-commit", game, state, 1)
            self.update("fraud-" + bad + "-finalize-rejected", game, moved(state), 2, valid=False)
            game = self.update("fraud-" + bad + "-challenge", game, fraud(state), 3, actor=1)
            self.exit("fraud-" + bad + "-settle", game, 4, winner=1)
        timeout = self.open_game("timeout-open")
        self.exit("timeout-missing-relative-proof", timeout, 5, since=0, valid=False)
        premature, inputs = self.exit_tx(timeout, 5, since=(1 << 63) | TIMEOUT)
        # Local VM validity does not establish consensus since maturity.
        self.vm("timeout-script-valid-but-immature", premature, inputs, True)
        try:
            self.rpc("send_transaction", [premature, "passthrough"])
        except RPCError as error:
            assert "Immature" in str(error), "timeout must fail actual consensus maturity"
            self.checks[-1]["node_maturity_rejection"] = error.error
        else:
            raise AssertionError("immature timeout accepted")
        self.mine(TIMEOUT + 1)
        self.exit("timeout-refund", timeout, 5, since=(1 << 63) | TIMEOUT)
        # Abort requires both actual participant lock groups, at the initial state.
        aborted = self.open_game("abort-open")
        inputs = [aborted] + self.wallets
        outputs = [output(STAKE, lock) for lock in self.player_locks] + [output(int(item["output"]["capacity"], 16) - (FEE if i == 0 else 0), self.player_locks[i]) for i, item in enumerate(self.wallets)]
        tx = sign_tx(new_tx(inputs, outputs, [b""] * 4, self.deps, 6), inputs, self.player_locks)
        self.submit("mutual-abort", tx, inputs)
        tip = self.rpc("get_tip_header")
        headers = []
        for height in range(int(tip["number"], 16) + 1):
            block = self.rpc("get_block_by_number", [hex(height)])
            self.save_block(block["header"]["hash"])
            headers.append(block["header"])
        write_json(self.evidence / "canonical-headers.json", headers)
        report = {"format": "myelin-ckb-microgame-drill-v1", "passed": True, "scope": "isolated Dummy-PoW CKB devnet; public test keys/capacity; strict full rule verifier; no mainnet or permissionless L2 claim", "genesis": self.genesis, "program_hash": self.program_hash, "schema_hash": hx(h(SCHEMA)), "vm_profile_hash": hx(h(VM)), "configured_depth": self.args.depth, "canonical_tip": tip["hash"], "canonical_height": int(tip["number"], 16), "receipts": self.receipts, "checks": self.checks, "moves": self.moves, "independent_da": {"status": "pending-external-provider-retrieval", "local_evidence_is_not_independent_da": True}}
        write_json(self.evidence / "report.json", report)
        return report

    def close(self):
        node = getattr(self, "node", None)
        if node and node.poll() is None:
            node.terminate()
            try:
                node.wait(timeout=10)
            except subprocess.TimeoutExpired:
                node.kill()
                node.wait()
        if hasattr(self, "log"):
            self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ckb", type=Path, required=True)
    parser.add_argument("--program", type=Path, required=True)
    parser.add_argument("--replayer", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--depth", type=int, default=2)
    args = parser.parse_args()
    assert 2 <= args.depth <= 16
    args.ckb = args.ckb.resolve(strict=True)
    drill = Drill(args)
    try:
        report = drill.run()
        print(json.dumps({"passed": report["passed"], "genesis": report["genesis"], "transactions": len(report["receipts"]), "script_checks": len(report["checks"]), "evidence": str(drill.evidence)}))
    finally:
        drill.close()


if __name__ == "__main__":
    main()
