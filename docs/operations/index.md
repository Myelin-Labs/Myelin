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

    The strict Rust microgame: actual deposits, proposals, challenge, settlement,
    timeout, abort and reorg transactions with retained offline replay.

-   [Workload measurements](workload-measurements.md)

    ---

    Reproduce three host-game workload scales, trigger comparisons, raw latency
    and resource samples, and fresh-process archive recovery.

-   [Downstream releases](downstream-release.md)

    ---

    Pin the framework source archive and build an independent game image from
    its own release package.

</div>

## What "running Myelin" looks like today

The workspace supplies synchronous execution/storage libraries, a CLI, a
Tokio producer and mTLS session/authority network services. Applications construct
their executor, genesis, trust policy, keys and service lifecycle. There is no
universal application daemon or automatic authority failover. See
[authority networking](../architecture/authority-network.md) for the service and
[writer migration](writer-migration.md) for ownership requirements.

## What "production evidence" looks like today

Use the single [validation entry point](production-gate.md) with an explicit
profile or lane. Each invocation retains `report.json`, exact commands, logs and
artifact hashes in its new output directory. Unselected integrations are marked
skipped; a selected lane with missing inputs fails. A core-only pass makes no
CKB, Teeworlds, browser or complete-build acceptance claim.
