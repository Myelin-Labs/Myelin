"""Measure host game rules, strict CellTx, durable commits and archive recovery.

The cases use development keys, not a throughput promise, complete build capsule
or independently deployed service benchmark. CellTx timing is not game timing.
"""
import argparse
from contextlib import nullcontext
import json
import math
import os
from pathlib import Path
import platform
import signal
import stat
import subprocess
import time

from validation import source_identity, file_identity, now
from workload_quota import QuotaVolume

SCALES = {"small": (1, 32), "medium": (4, 128), "large": (8, 512)}
KINDS = ("inventory", "turns", "ticks", "cells", "rewards")
TRIGGERS = ("instant", "interval", "open", "manual")
FAULTS = ("process-exit", "slow-executor", "unknown-results", "cancel-refund", "reward-process-exit",
          "evidence-slow", "evidence-offline", "disk-quota")


def write(path, value):
    pending = path.with_suffix(path.suffix + ".pending")
    pending.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    pending.replace(path)


def tree_size(path):
    logical = allocated = files = 0
    inodes = set()
    kinds = {}
    if not path.exists():
        return None
    for base, _, names in os.walk(path):
        for name in names:
            info = (Path(base) / name).lstat()
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f"non-regular measurement artifact: {base}/{name}")
            logical += info.st_size
            files += 1
            suffix = Path(name).suffix
            kind = ({".log": "rocksdb_wal", ".sst": "rocksdb_table"}.get(suffix, "other")
                    if Path(name).stem.isdecimal() else "other")
            group = kinds.setdefault(kind, {"logical_file_bytes": 0, "allocated_file_bytes": 0, "files": 0})
            group["logical_file_bytes"] += info.st_size
            group["files"] += 1
            identity = (info.st_dev, info.st_ino)
            if identity not in inodes:
                inodes.add(identity)
                allocated += info.st_blocks * 512
                group["allocated_file_bytes"] += info.st_blocks * 512
    return {"logical_file_bytes": logical, "allocated_file_bytes": allocated, "files": files, "file_types": kinds}


def rss(pid):
    try:
        values = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines() if ":" in line)
        return int(values["VmRSS"].split()[0]) * 1024
    except (FileNotFoundError, ProcessLookupError, KeyError):
        return None


def percentile(values):
    values = sorted(values)
    if not values:
        return None
    return {"n": len(values), "min": values[0], "p50": values[math.ceil(len(values) * .5) - 1],
            "p95": values[math.ceil(len(values) * .95) - 1], "p99": values[math.ceil(len(values) * .99) - 1],
            "max": values[-1], "mean": sum(values) / len(values)}


def process_tree(pid):
    """Include providers spawned by any thread, not just the process leader."""
    pending, seen = [pid], set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        try:
            for task in Path(f"/proc/{current}/task").iterdir():
                try:
                    pending.extend(map(int, (task / "children").read_text().split()))
                except (FileNotFoundError, ProcessLookupError):
                    pass
        except (FileNotFoundError, ProcessLookupError):
            pass
    return seen


def run_group(binary, folder, phase, jobs, timeout, expected_exit=0, observed_pids=()):
    """wait4 returns per-child usage; no cumulative RUSAGE_CHILDREN peak subtraction."""
    active, completed = [], []
    start = time.monotonic()
    sampled_peak = 0
    with (folder / f"{phase}-rss.jsonl").open("w") as samples:
        try:
            for index, args in enumerate(jobs):
                stdout = folder / f"{phase}-{index}.json"
                stderr = folder / f"{phase}-{index}.stderr"
                with stdout.open("wb") as out, stderr.open("wb") as err:
                    argv = [str(binary), *map(str, args)]
                    process = subprocess.Popen(argv, stdout=out, stderr=err, start_new_session=True)
                active.append((index, process, argv, stdout, stderr))
            while active:
                pids = set(observed_pids).union(*(process_tree(p.pid) for _, p, *_ in active))
                readings = {str(pid): rss(pid) for pid in sorted(pids)}
                sampled_peak = max(sampled_peak, sum(v for v in readings.values() if v is not None))
                samples.write(json.dumps({"elapsed_seconds": time.monotonic() - start, "rss_bytes": readings}) + "\n")
                for item in list(active):
                    index, process, argv, stdout, stderr = item
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if not pid:
                        continue
                    process.returncode = os.waitstatus_to_exitcode(status)
                    active.remove(item)
                    result = {"index": index, "argv": argv, "exit_code": process.returncode,
                              "elapsed_since_group_start_seconds": time.monotonic() - start,
                              "user_cpu_seconds": usage.ru_utime, "system_cpu_seconds": usage.ru_stime,
                              "lifetime_peak_rss_bytes": usage.ru_maxrss * 1024,
                              "stdout": stdout.name, "stderr": stderr.name}
                    completed.append(result)
                    write(folder / f"{phase}-processes.json", completed)
                    if process.returncode != expected_exit:
                        raise RuntimeError(f"{phase} exited {process.returncode}: {stderr.read_text()[-4000:]}")
                if time.monotonic() - start > timeout:
                    raise TimeoutError(f"{phase} exceeded {timeout} seconds")
                if active:
                    time.sleep(.01)
        finally:
            for _, process, *_ in active:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                _, status, _ = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
    result = {"elapsed_seconds": time.monotonic() - start, "sampled_group_peak_rss_bytes": sampled_peak,
              "rss_sampling_interval_ms": 10, "rss_scope": "workers, live descendants and explicitly observed filesystem daemons",
              "observed_pids": list(observed_pids),
              "processes": sorted(completed, key=lambda x: x["index"])}
    write(folder / f"{phase}-resources.json", result)
    return result


def latency(roots, kind):
    cells, rewards = kind == "cells", kind == "rewards"
    rows, batches = [], []
    for root in roots:
        for path in root.glob("transactions-*.jsonl" if cells else "rewards-*.jsonl" if rewards else "commands-*.jsonl"):
            rows.extend(json.loads(line) for line in path.read_text().splitlines())
        for path in root.glob("batches-*.jsonl"):
            batches.extend(json.loads(line) for line in path.read_text().splitlines())
    identities = set()
    for row in rows:
        key = (row["pair" if rewards else "session"], row["sequence"])
        if key in identities:
            raise ValueError("duplicate measured command")
        identities.add(key)
        stamp_keys = (("scheduled_ns", "admitted_ns", "simulation_finished_ns", "issued_ns", "resolved_ns") if rewards else
                     ("scheduled_ns", "admitted_ns", "simulation_finished_ns", "committed_ns") if cells else
                      ("scheduled_ns", "enqueue_started_ns", "enqueued_ns", "simulation_finished_ns", "committed_ns", "response_ns"))
        stamps = [row[k] for k in stamp_keys]
        if stamps != sorted(stamps):
            raise ValueError("latency timestamps are not monotonic")
        if rewards and ((row["outcome"] == "credited") != (row["credited_ns"] is not None) or
                        row["credited_ns"] is not None and not row["issued_ns"] <= row["credited_ns"] <= row["resolved_ns"]):
            raise ValueError("reward outcome/timing differs")
    count_key = "transactions" if cells else "commands"
    if sum(b[count_key] for b in batches) != len(rows) * (3 if rewards else 1) or any((b["vm_cycles"] > 0) != cells for b in batches):
        raise ValueError("batch counts or execution cycle classification differ")
    metrics = {
        "input_to_simulation_ns": ("simulation_finished_ns", "scheduled_ns"),
        "input_to_admission_ns": ("admitted_ns" if cells or rewards else "enqueue_started_ns", "scheduled_ns"),
    }
    phases = ["prepare_ns", "local_poa_sign_ns", "verify_and_sync_commit_ns"]
    if rewards:
        metrics.update(input_to_issue_commit_ns=("issued_ns", "scheduled_ns"), input_to_inventory_credit_ns=("credited_ns", "scheduled_ns"),
                       input_to_terminal_ns=("resolved_ns", "scheduled_ns"))
        phases += ["local_outbox_ack_ns", "evidence_enqueue_ns"]
    else:
        metrics["input_to_durable_commit_ns"] = ("committed_ns", "scheduled_ns")
    if not cells and not rewards:
        metrics.update(input_to_response_ns=("response_ns", "scheduled_ns"), enqueue_ns=("enqueued_ns", "enqueue_started_ns"),
                       queued_until_simulation_ns=("simulation_finished_ns", "enqueued_ns"))
        phases += ["reserve_ns", "reconcile_and_ack_ns"]
    def summarize(selected):
        return {name: percentile([r[a] - r[b] for r in selected if r[a] is not None]) for name, (a, b) in metrics.items()}
    def phases_for(selected):
        return {name: percentile([b[name] for b in selected]) for name in phases}
    key = "pair" if rewards else "session"
    return {"inputs": len(rows), "input_kind": "rewards" if rewards else count_key, "blocks": len(batches), "aggregate": summarize(rows),
            "outcomes": {outcome: sum(r.get("outcome") == outcome for r in rows) for outcome in ("credited", "refunded")} if rewards else None,
            "sessions": {str(i): summarize([r for r in rows if r[key] == i]) for i in sorted({r[key] for r in rows})},
            "batch_input_kind": count_key, "committed_commands": sum(b[count_key] for b in batches),
            "batch_inputs": percentile([b[count_key] for b in batches]),
            "vm_cycles": {"total": sum(b["vm_cycles"] for b in batches), "per_batch": percentile([b["vm_cycles"] for b in batches])},
            "batch_phases_ns": phases_for(batches),
            "reward_stages_ns": {stage: phases_for([b for b in batches if b["phase"] == stage])
                                 for stage in sorted({b["phase"] for b in batches})} if rewards else None,
            "executor_costs_ns": {name: percentile([b["execution"][name] for b in batches]) for name in
                                  ("outer_clone_ns", "execute_ns", "snapshot_ns")}}


def storage(roots):
    return [{"canonical": tree_size(root / "canonical"), "journals": {p.name: tree_size(p) for p in root.glob("journal-*")},
             "providers": {p.name: tree_size(p) for p in root.glob("provider-*") if p.is_dir()},
             "archive": tree_size(root / "archive")} for root in roots]


def evidence_measurements(roots, pairs, rewards, fault):
    rows, summaries, requests, inputs = [], [], [], []
    for root in roots:
        for path in root.glob("rewards-*.jsonl"):
            inputs.extend(json.loads(line) for line in path.read_text().splitlines())
        for path in root.glob("evidence-*.jsonl"):
            rows.extend(json.loads(line) for line in path.read_text().splitlines())
        summaries.extend(json.loads(path.read_text()) for path in root.glob("evidence-summary-*.json"))
        for path in root.glob("provider-*/requests.jsonl"):
            requests.extend(json.loads(line) for line in path.read_text().splitlines())
    failures = [r for r in rows if r["status"] == "failed"]
    successful = [r for r in rows if r["status"] != "failed"]
    complete = [r for r in successful if r["status"] == "complete"]
    expected_failures = int(fault == "evidence-offline")
    if (len(failures) != expected_failures or len(complete) != pairs * rewards * 3 or
            len(successful) != len(complete) * 2 or len(summaries) != pairs or
            {s["pair"] for s in summaries} != set(range(pairs)) or
            any(s["complete"] != rewards * 3 for s in summaries)):
        raise ValueError("evidence completion/failure counts differ from reward outbox")
    messages = {}
    for row in rows:
        if not row["committed_ns"] <= row["started_ns"] <= row["finished_ns"]:
            raise ValueError("evidence timing precedes canonical commit")
        if min(row["costs"].values()) < 0 or row["evidence_store_and_ack_ns"] < 0:
            raise ValueError("invalid evidence stage timing")
        key = (row["pair"], row["message"])
        if row["status"] != "failed":
            messages.setdefault(key, []).append(row["stage"])
    if any(stages != ["provider-stored", "retrieval-verified"] for stages in messages.values()) or len(messages) != len(complete):
        raise ValueError("evidence stage chain missing or duplicated")
    publications = {(r["pair"], r["message"]): r["finished_ns"] for r in complete}
    expected = set()
    end_to_end = []
    for item in inputs:
        ids = item["outbox_messages"]
        if len(ids) != 3:
            raise ValueError("reward lacks its three exact publication identities")
        keys = [(item["pair"], message) for message in ids]
        if expected.intersection(keys):
            raise ValueError("reward publication identity reused")
        expected.update(keys)
        stamps = [publications[k] for k in keys]
        end_to_end.append({"pair": item["pair"], "inventory_ns": stamps[1] - item["scheduled_ns"],
                           "terminal_ns": max(stamps) - item["scheduled_ns"]})
    if expected != publications.keys() or len(inputs) != pairs * rewards:
        raise ValueError("publication does not cover every original reward")
    if fault == "evidence-offline":
        restarts = [json.loads(p.read_text()) for root in roots for p in root.glob("provider-restart-*.json")]
        if len(restarts) != 1 or not restarts[0]["canonical_progress_while_offline"] or not restarts[0]["connection_refused"]:
            raise ValueError("provider outage did not preserve canonical progress")
        if sum(r["stage"] == "provider-stored" and r["reused_durable_blob"] for r in requests) != 1:
            raise ValueError("provider restart did not reuse exactly the unknown-outcome durable blob")
    def summarize(selected):
        return {"commit_to_publication_ns": percentile([r["finished_ns"] - r["committed_ns"] for r in selected if r["status"] == "complete"]),
                "proof_collection_ns": percentile([r["costs"]["collect_ns"] for r in selected]),
                "local_verification_ns": percentile([r["costs"]["verify_ns"] for r in selected if r["status"] != "failed"]),
                "evidence_store_and_ack_ns": percentile([r["evidence_store_and_ack_ns"] for r in selected if r["status"] != "failed"])}
    return {"provider_scope": "separate loopback processes; signed durable blob and retrieval; development keys; no public DA quorum",
            "messages": len(complete), "failed_attempts": len(failures), "aggregate": summarize(rows),
            "pairs": {str(i): summarize([r for r in rows if r["pair"] == i]) | {
                "input_to_inventory_evidence_ns": percentile([r["inventory_ns"] for r in end_to_end if r["pair"] == i]),
                "input_to_terminal_evidence_ns": percentile([r["terminal_ns"] for r in end_to_end if r["pair"] == i])}
                for i in range(pairs)}, "workers": summaries}


def head_map(folder, phase, count, first=0):
    result = {}
    for index in range(first, first + count):
        for session in json.loads((folder / f"{phase}-{index}.json").read_text())["sessions"]:
            if session["index"] in result:
                raise ValueError("duplicate session in phase results")
            result[session["index"]] = (session["head"], session["state"])
    return result


def case(binary, folder, kind, scale, trigger, layout, timeout, seed, fault, arrival_us, batch, window_us):
    cells, rewards = kind == "cells", kind == "rewards"
    prefix = [kind] if cells or rewards else []
    crashed = fault in ("process-exit", "reward-process-exit")
    count, commands = SCALES[scale] if scale != "long" else (1, 4096)
    if kind == "turns":
        commands = 9
    folder.mkdir()
    plan = {"kind": kind, "sessions": count, "first_session": 0, "commands": commands, "batch": batch,
            "seed": seed, "trigger": trigger, "arrival_us": arrival_us, "window_us": window_us,
            "slow_first_session_us": 2000 if fault == "slow-executor" else 0,
            "unknown_every_blocks": 11 if fault == "unknown-results" else 0,
            "crash_after_first_commit": fault == "process-exit"}
    if scale == "long":
        plan.update(batch=1, arrival_us=0)
    if crashed:
        count = plan["sessions"] = 1
        plan.update(batch=1, arrival_us=0)
    if cells:
        if fault:
            raise ValueError("host command fault modes do not apply to the CellTx workload")
        plan = {k: plan[k] for k in ("sessions", "first_session", "batch", "seed", "trigger", "arrival_us", "window_us")}
        plan["transactions"] = commands
    if rewards:
        plan = {k: plan[k] for k in ("batch", "seed", "trigger", "arrival_us", "window_us")}
        plan.update(pairs=count, first_pair=0, rewards=commands, cancel_every=11 if fault == "cancel-refund" else 0,
                    crash_after_credit=fault == "reward-process-exit",
                    evidence={"evidence-slow": "slow", "evidence-offline": "offline"}.get(fault, "disabled"))
    group_key, first_key = ("pairs", "first_pair") if rewards else ("sessions", "first_session")
    plans = [plan] if layout == "shared" else [plan | {group_key: 1, first_key: i,
        **({} if cells or rewards else {"slow_first_session_us": plan["slow_first_session_us"] if i == 0 else 0})} for i in range(count)]
    roots, jobs = [], []
    for index, item in enumerate(plans):
        path = folder / f"plan-{index}.json"
        write(path, item)
        root = folder / f"worker-{index}"
        roots.append(root)
        jobs.append([*prefix, "run", path, root])
    result = {"kind": kind, "scale": scale, "trigger": trigger, "layout": layout, "fault": fault, "plan": plan, "status": "running"}
    write(folder / "case.json", result)
    with QuotaVolume(folder, roots[0]) if fault == "disk-quota" else nullcontext() as quota:
        observed = quota.pids if quota else []
        if quota:
            jobs[0] = [*prefix, "quota", folder / "plan-0.json", roots[0]]
            for i in range(1, len(jobs)):
                jobs[i] = [*prefix, "run-after-quota", folder / f"plan-{i}.json", roots[i], roots[0] / "fault-active.json"]
        result["run"] = run_group(binary, folder, "run", jobs, timeout, 76 if crashed else 0, observed)
        result["produced_storage"] = storage(roots)
        if fault in ("evidence-slow", "evidence-offline"):
            result["evidence"] = evidence_measurements(roots, count, commands, fault)
        if quota:
            failed = json.loads((roots[0] / "quota-fault.json").read_text())
            if len(failed["failed_writes"]) != plans[0]["pairs"]:
                raise ValueError("quota did not reject writes in every session sharing the affected store")
            result["quota_fault"] = failed
            if len(roots) > 1:
                result["unaffected_latency"] = latency(roots[1:], kind)
                if result["unaffected_latency"]["inputs"] != commands * (count - 1):
                    raise ValueError("independent services did not finish while quota remained exhausted")
            quota.release_and_remount()
            observed = quota.pids
            result["quota_resume"] = run_group(binary, folder, "quota-resume", [[*prefix, "quota-recover", roots[0]]], timeout, observed_pids=observed)
            result["quota_reconciliation"] = json.loads((roots[0] / "quota-reconciled.json").read_text())
            result["storage_after_quota_recovery"] = storage(roots)
        if not crashed and not quota:
            result["latency"] = latency(roots, kind)
            if result["latency"]["inputs"] != commands * count:
                raise ValueError("workload completed with missing command samples")
            result["process_throughput_inputs_per_second"] = commands * count / result["run"]["elapsed_seconds"]
        result["recovery"] = run_group(binary, folder, "recover", [[*prefix, "recover", root] for root in roots], timeout, observed_pids=observed)
        recovered = head_map(folder, "recover", len(roots))
        if set(recovered) != set(range(count * (2 if rewards else 1))):
            raise ValueError("recovery did not return the exact selected sessions")
        if fault == "process-exit":
            report = json.loads((folder / "recover-0.json").read_text())["sessions"][0]
            if report["head"]["input_position"] != 1 or report["reconciled_entries"] != 1:
                raise ValueError("process-exit drill did not recover exactly one committed command")
        else:
            if quota:
                produced = head_map(folder, "quota-resume", 1)
                neighbors = head_map(folder, "run", len(roots) - 1, 1)
                if produced.keys() & neighbors.keys():
                    raise ValueError("quota recovery duplicated an independent session")
                produced.update(neighbors)
            elif fault == "reward-process-exit":
                if json.loads((folder / "recover-0.json").read_text())["reconciled_rewards"] != 1:
                    raise ValueError("reward crash did not reconcile exactly one unknown credit")
                produced = recovered
            else:
                produced = head_map(folder, "run", len(roots))
            if produced != recovered:
                raise ValueError("restart changed heads or game states")
            result["export"] = run_group(binary, folder, "archive", [[*prefix, "archive", root] for root in roots], timeout, observed_pids=observed)
            result["archived_storage"] = storage(roots)
            # Only these newly created benchmark databases belong to this deletion
            # scope. The retained archive and reports must recover without them.
            import shutil
            for root, item in zip(roots, plans):
                if quota and root == roots[0]:
                    quota.remove()
                    result["quota_filesystem"] = json.loads((folder / "quota-filesystem.json").read_text())
                else:
                    shutil.rmtree(root / "canonical")
                if not cells and not rewards:
                    for i in range(item["first_session"], item["first_session"] + item["sessions"]):
                        shutil.rmtree(root / f"journal-{i}")
            result["cold_recovery"] = run_group(binary, folder, "cold", [[*prefix, "cold", root] for root in roots], timeout)
            if produced != head_map(folder, "cold", len(roots)):
                raise ValueError("archive replay changed heads or game states")
        result["status"] = "passed"
        write(folder / "case.json", result)
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--scales", nargs="+", choices=SCALES, default=list(SCALES))
    parser.add_argument("--kinds", nargs="+", choices=KINDS, default=list(KINDS))
    parser.add_argument("--triggers", nargs="+", choices=TRIGGERS, default=list(TRIGGERS))
    parser.add_argument("--layouts", nargs="+", choices=("shared", "services"), default=["shared", "services"])
    parser.add_argument("--seed", type=int, default=17)
    parser.add_argument("--arrival-us", type=int, default=1000, help="scheduled input interval per session or reward pair")
    parser.add_argument("--batch", type=int, default=8, help="per-frame input ceiling (reward issuance batch size)")
    parser.add_argument("--window-us", type=int, default=5000, help="Interval/Open/manual window")
    parser.add_argument("--timeout-seconds", type=int, default=3600)
    parser.add_argument("--drills", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--faults", nargs="+", choices=FAULTS, help="select only these fault drills for the selected workloads")
    args = parser.parse_args()
    if args.faults and not args.drills:
        parser.error("--faults requires drills; remove --no-drills")
    if not 0 <= args.seed < 2**64 or not 1 <= args.timeout_seconds <= 14400:
        parser.error("seed/timeout outside supported bounds")
    if not 0 <= args.arrival_us <= 1_000_000 or not 1 <= args.batch <= 64 or not 1 <= args.window_us <= 1_000_000:
        parser.error("arrival/batch/window outside supported bounds")
    if platform.system() != "Linux":
        parser.error("measurement sampling and child resource accounting require Linux")
    root = Path(__file__).resolve().parents[1]
    out = args.out.resolve()
    if out.is_relative_to(root) and not out.is_relative_to(root / "target"):
        parser.error("output must be outside source or below target/")
    out.mkdir(parents=True, exist_ok=False)
    report = {"schema": "myelin-workload-measurements-v2", "status": "running", "started_at": now(), "source": source_identity(root),
              "selection": {key: getattr(args, key) for key in ("scales", "kinds", "triggers", "layouts", "seed", "drills", "faults", "arrival_us", "batch", "window_us")},
              "machine": {"platform": platform.platform(), "architecture": platform.machine(), "logical_cpus": os.cpu_count(),
                          "affinity": sorted(os.sched_getaffinity(0)), "memory": Path("/proc/meminfo").read_text(),
                          "cpu": subprocess.check_output(["lscpu"], text=True)},
              "toolchain": {"rustc": subprocess.check_output(["rustc", "-vV"], text=True),
                            "cargo": subprocess.check_output(["cargo", "-V"], text=True)},
              "environment": {k: os.environ[k] for k in ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "RUSTFLAGS", "RUSTUP_TOOLCHAIN") if k in os.environ},
              "cases": [], "teeworlds": {"status": "skipped", "reason": "separate existing external acceptance lane; not selected by this host workload command"},
              "not_measured": ["game logic inside strict CKB-VM", "remote consensus quorum proof collection", "public-network evidence providers", "physical multi-host fencing",
                               "disk quota faults", "platform-index outages"],
              "cold_scope": "fresh process and original databases removed; OS page cache is not purged"}
    write(out / "report.json", report)
    try:
        build = ["cargo", "build", "--locked", "--release", "-p", "myelin-session-journal", "--example", "workloads"]
        report["build_command"] = build
        print("building release workload executable", flush=True)
        with (out / "build.log").open("wb") as log:
            process = subprocess.Popen(build, cwd=root, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                if process.wait(timeout=3600):
                    raise RuntimeError(f"release build failed; see {out / 'build.log'}")
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root))
        binary = Path(metadata["target_directory"]) / "release/examples/workloads"
        report["binary"] = {"path": str(binary), **file_identity(binary)}
        cases = [(kind, scale, trigger, layout, None) for scale in dict.fromkeys(args.scales) for kind in dict.fromkeys(args.kinds)
                 for trigger in dict.fromkeys(args.triggers) for layout in dict.fromkeys(args.layouts)]
        if args.drills and "ticks" in args.kinds:
            cases += [("ticks", "long", "instant", "shared", None), ("ticks", "small", "instant", "shared", "process-exit")]
            cases += [("ticks", "large", "instant", layout, fault) for layout in dict.fromkeys(args.layouts) for fault in ("slow-executor", "unknown-results")]
        if args.drills and "rewards" in args.kinds:
            cases += [("rewards", "medium", "instant", layout, "cancel-refund") for layout in dict.fromkeys(args.layouts)]
            cases += [("rewards", "small", "instant", "shared", "reward-process-exit")]
            cases += [("rewards", "medium", "instant", layout, fault) for layout in dict.fromkeys(args.layouts)
                      for fault in ("evidence-slow", "evidence-offline", "disk-quota")]
        if args.faults:
            missing = set(args.faults) - {c[4] for c in cases}
            if missing:
                raise ValueError(f"faults do not apply to selected workloads: {sorted(missing)}")
            cases = [c for c in cases if c[4] in args.faults or c[4] is None and c[1] != "long"]
        for kind, scale, trigger, layout, fault in cases:
            name = "-".join((kind, scale, trigger, layout, fault or "normal"))
            report["running_case"] = name
            write(out / "report.json", report)
            print(name, flush=True)
            result = case(binary, out / name, kind, scale, trigger, layout, args.timeout_seconds, args.seed, fault,
                          args.arrival_us, args.batch, args.window_us)
            report["cases"].append({"name": name, "report": f"{name}/case.json", "status": result["status"]})
            write(out / "report.json", report)
        report["source_after"] = source_identity(root)
        if report["source"] != report["source_after"] or file_identity(binary) != {k: v for k, v in report["binary"].items() if k != "path"}:
            raise ValueError("source or executable changed during measurement")
        if any(c[4] == "disk-quota" for c in cases):
            report["not_measured"].remove("disk quota faults")
        report["status"] = "passed"
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["finished_at"] = now()
        report["artifacts"] = {str(p.relative_to(out)): file_identity(p) for p in sorted(out.rglob("*")) if p.is_file() and p != out / "report.json"}
        write(out / "report.json", report)
        print(json.dumps({"status": report["status"], "report": str(out / "report.json")}), flush=True)


if __name__ == "__main__":
    main()
