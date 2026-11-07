`always_success` is the 344-byte CKB integration-test fixture distributed by
[Nervos CKB](https://github.com/nervosnetwork/ckb/blob/d40e4722525d45d5ce7a8ae29a1dc6d18d2f747f/test/template/specs/cells/always_success)
under that project's [MIT license](https://github.com/nervosnetwork/ckb/blob/d40e4722525d45d5ce7a8ae29a1dc6d18d2f747f/COPYING).
The local source file matched Git blob `248e53c5a08dd0483754ad729e4c597c0c78b076`.
SHA-256: `447282f65719c67044506f25103fc11325a1776d67cc06784b81f7c665c8b851`.
CKB code hash: `28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5`.
The drill uses this public lock with the strict game type script; it is not an
authorization mechanism for player inputs or a substitute for game verification.

The devnet specification is an intentionally authored test configuration based
on CKB's integration fixture. Its block cycle ceiling is 10,000,000, matching the
strict replay budget. It is not a public-testnet or mainnet specification.
