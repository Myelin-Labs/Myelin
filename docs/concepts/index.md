# Concepts

This section explains the building blocks Myelin sits on top of: the
**Cell Model** that CKB introduced, the **CKB-VM** that runs the
scripts, and the **identity Myelin itself takes** when it borrows
from those primitives.

If you already know CKB well, you can skim
[What is CKB?](what-is-ckb.md) and jump straight to
[What is Myelin?](what-is-myelin.md). If you've never met CKB before,
read them in order.

## Pages

<div class="grid cards" markdown>

-   [What is CKB?](what-is-ckb.md)

    ---

    The Cell Model, transactions, scripts, capacity, witnesses,
    and why CKB is UTXO-shaped instead of account-shaped.

-   [What is CKB-VM?](what-is-ckb-vm.md)

    ---

    RISC-V, deterministic execution, syscalls, cycle accounting,
    and why scripts run in a VM instead of a co-located process.

-   [What is Myelin?](what-is-myelin.md)

    ---

    How Myelin re-uses the Cell mental model for off-chain sessions
    while keeping a CKB-style projection path.

-   [Semantic profiles](semantic-profiles.md)

    ---

    The three profiles (`ckb-compatible`, `myelin-native`,
    `ckb-inspired-only`) and how they shape what Myelin will
    and won't claim about a transition.

</div>

## Why these three pages exist

Most L2 / scaling literature assumes the reader already knows the L1
well. Myelin's docs deliberately don't. The Cell Model, the VM, and
the Myelin identity are all genuinely different from the
EVM/account/smart-contract pattern that dominates crypto docs. If you
read [What is CKB?](what-is-ckb.md) and [What is CKB-VM?](what-is-ckb-vm.md)
once, the rest of the architecture docs become obvious.