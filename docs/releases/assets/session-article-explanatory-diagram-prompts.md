# Explanatory diagram prompt set

Mode: OpenAI ImageGen, reference-guided infographic redesign.

Shared direction: create a publication-quality flat 2D systems diagram for a technical article. Preserve the supplied reference's semantics while redesigning its hierarchy, alignment, typography and flow. Use a wide, contemporary neo-grotesk sans serif with ordinary humanist proportions, similar to Inter or IBM Plex Sans. Never use condensed, compressed, tall or narrow display type. Use only deep navy (`#0B1F2A`), warm ivory (`#F4F0E8`), muted teal (`#55BFC0`) and tonal variants. Use arrows only when they encode timing, dependency or flow. Keep every label short, exact and inside its bounds. Avoid conceptual artwork, 3D or isometric objects, scenery, glossy surfaces, decorative textures, generic dashboards, pseudo-text, logos and watermarks.

## From one verified chunk to a session

Landscape explanatory flow. Title: 'FROM ONE VERIFIED CHUNK TO A SESSION'. Subtitle: 'A valid transition needs a history'. Show `RECORDED INPUTS → CKB-VM → STATE ROOT S37`, then a continuous block lineage `37 → 38 → 39 → HEAD` with state lineage `S37 → S38 → S39` and supporting labels `ORDERING`, `FINALITY`, `STORAGE`, `RECOVERY`. Finish with an ascending evidence ladder `WIRE BYTES → LINKED ADAPTER RECEIPTS → HIGHER CKB CLAIMS`, with `context · scripts · node · commitment · depth` beneath receipts. Make higher claims visibly depend on linked receipts; never imply that wire bytes alone are accepted, committed or final.

## Four block-production triggers

Landscape timing chart. Title: 'FOUR BLOCK-PRODUCTION TRIGGERS'. Subtitle: 'Shared families, one deliberate Open adaptation'. Use four aligned horizontal lanes. `INSTANT`: a `TX AVAILABLE` dot followed immediately by `PRODUCE`; note `ON AVAILABILITY`. `INTERVAL`: fixed ticks `T`, `T+Δ`, `T+2Δ`, including one empty interval; note `CADENCE · EMPTY ALLOWED`. `OPEN`: note `STARTS IMMEDIATELY`, then draw a bracket from `START NOW` to `PERIOD DEADLINE`, accumulating transaction dots. Beneath the lane, show two clearly separated annotations: `FUEL CORE · COMPLETE AT DEADLINE` and `MYELIN · DEADLINE OR BATCH LIMIT`. Never imply that a Fuel Core batch limit is an early trigger. `NEVER`: no automatic production, then `MANUAL REQUEST` followed by `PRODUCE`; note `AUTOMATIC OFF`. Beneath the lanes show only the Myelin hand-off: `FIXED BATCH → EXECUTE → FINALITY → ATOMIC COMMIT`, labelled `MYELIN AFTER THE TRIGGER`. Footer: 'MYELIN SUPPORTS ALL FOUR'.

## Genesis-locked modules

Landscape architecture flowchart. Title: 'SELECT AT GENESIS. VERIFY ON RECOVERY.'. Subtitle: 'A pluggable module keeps one stable identity'. Stage 1, `CLOSED CATALOGUE`: `COMMITTEE`, `POA`, `TENDERMINT`, followed by `SELECT ONE`. Stage 2, `GENESIS COMMITS`: one module identity record containing `KIND`, `CONFIG`, `MODULE`, `WAL SCHEMA`. Stage 3, `SAME MODULE ID BINDS`: link the identity to `PROOFS`, `NETWORK`, `WAL`, `BLOCK RECORDS`. Stage 4, `RECOVERY CHECK`: branch `MATCH → WRITER OPENS` and `MISMATCH → WRITER CLOSED`, with the mismatch branch emphasised. Footer: 'SELECTABLE BETWEEN SESSIONS · FIXED WITHIN ONE SESSION'.
