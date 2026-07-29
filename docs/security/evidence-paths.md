# Evidence paths

Myelin keeps independent claims on separate paths:

| Path | Current evidence | Where it stops |
| --- | --- | --- |
| Execution | strict VM group results, shared cycle count, exact pre/post state roots; optional context-bound CKB adapter receipt | Myelin execution alone is local; adapter execution is bound to an exact resolved CKB context |
| Scheduling | raw-tx-bound logical accesses, DAG order/results, scheduler commitment | sidecar evidence; not a CKB witness or consensus field |
| Projection | wire report plus linked context/consensus/scripts/node/commitment/finality receipts | pure projector stops at `wire-encoded`; adapter can reach `finalized` for one exercised transaction |
| Finality | configured quorum signatures over canonical `MyelinBlock` | closed validator set only |
| DA | segment proof, local storage/attestations, optional external receipt | local publication is not L1 publication |
| Court | disputed-chunk package plus internal verifier checks | input shape only; `court_verifiable = false` |
| Settlement | authority/economics/DA-bound package and RPC request | no deployed L1 court or verdict by default |
| Submission | full-node pre-admission, exact-hash send, and non-committed transaction observation | acceptance is not commitment; commitment/finality use separate evidence |

Evidence is compositional only when hashes and lineage match. A stronger path cannot be inferred from a weaker one: wire encoding does not imply execution validity, local VM success does not imply contextual CKB validity, and a signed Myelin block does not imply L1 acceptance.

The projection receipt chain is described in the repository-level `MYELIN_CKB_PROJECTION_AUDIT.md`.
