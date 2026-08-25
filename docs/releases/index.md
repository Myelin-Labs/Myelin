# Releases & positioning

Release-positioning notes that frame what Myelin is, where it sits in its
research line, and what each release does and does not claim. These are
framing documents, not security claims — for the latter, see the
[claim ladder](../security/claim-ladder.md).

- [Myelin 0.21.0 release record](myelin-0.21.0.md) — application identity,
  execution frames, inspection and replay, crash-resumable evidence ladders,
  successor sealing, and single-use handoffs.
- [Myelin 0.20.0 release record](myelin-0.20.0.md) — the reviewed release
  boundary for the modular continuous-session runtime, including the exact
  commit range, validation commands, claim limits, and upgrade note.
- [Introducing Myelin (Nervos Talk draft)](nervos-talk-introducing-myelin.md) —
  the canonical public introduction: what Myelin is, the five things it adds
  above a CKB-VM-verified chunk, the research line it continues (xuejie's
  Teeworlds / fat-thin / OHOL / Archipelago posts), the roadmap those posts
  sharpened, and the hard security boundary.
- [From bounded sessions to continuous operation: pluggable chain modules in Myelin](nervos-talk-pluggable-session-chain.md) —
  a revised version of the
  [Nervos Talk post](https://talk.nervos.org/t/from-bounded-sessions-to-continuous-operation-pluggable-chain-modules-in-myelin/10658),
  covering one-candidate production, genesis-bound finality, checkpoint
  recovery, and an independent Veloren research fork. Its `Instant`,
  `Interval`, `Open`, and `Never` vocabulary comes from studying Fuel Core's
  block-production service; Myelin gives `Open` an availability-started window.
