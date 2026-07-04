# Operations

This section is for the person running Myelin: installing it,
running it, and proving it works against a real (or mock) CKB node.

## Pages

<div class="grid cards" markdown>

-   [CLI reference](cli.md)

    ---

    Every `myelin-cli` subcommand, what it takes, what it produces.

-   [Production gate](production-gate.md)

    ---

    The `scripts/myelin_production_gate.sh` end-to-end script:
    what it runs, in what order, what each step proves.

-   [Local CKB devnet smoke](devnet-smoke.md)

    ---

    The `scripts/myelin_ckb_devnet_smoke.sh` script: live carrier
    submissions, type-script execution, and live rejection of
    tampered carriers on a parent CKB devnet.

</div>

## What "running Myelin" looks like today

There is **no daemon**. There is **no P2P layer**. There is **no
RPC server**. The way you run Myelin today is:

1. **Compile the CLI.**
2. **Run subcommands** that produce and verify reports on disk.
3. **Run scripts** that orchestrate the subcommands and assert
   properties of the reports.

If you want a daemon or RPC server, that's a separate piece of work
— Myelin today is a kernel + CLI, not a service.

## What "production evidence" looks like today

Two scripts in `scripts/` produce the evidence:

```bash
scripts/myelin_production_gate.sh       # full local gate
scripts/myelin_teeworlds_acceptance.sh  # narrower Teeworlds gate
scripts/myelin_ckb_devnet_smoke.sh      # live CKB devnet path
```

Each one leaves behind a set of JSON reports in `reports/` that
record what was measured, what was projected, and what was
submitted. Those reports are the evidence; the CLI exit codes are
the gate.

## Where to go next

- [CLI reference](cli.md) — pick the subcommand you need.
- [Production gate](production-gate.md) — run the whole thing.
- [Local CKB devnet smoke](devnet-smoke.md) — if you have a local
  CKB devnet and want live evidence.