# Grey Goo

A fully on-chain, closed-system **evolutionary artificial-life simulation** for Solana. Nanotech-themed, but there is no real-world data and no oracles — the chain *is* the world. Autonomous agents ("nanobots") live on a toroidal grid, consume a scarce regenerating resource, replicate with mutated genomes, compete, and evolve. The world is advanced by a permissionless `tick`.

The design rests on two lines of established prior art: **Avida / Tierra** show that self-replicating programs competing for a scarce resource in a closed, deterministic system produce real selection, speciation, and adaptive radiation; **Dark Forest / MUD / MagicBlock** show that fully-on-chain game state is viable. Grey Goo joins the two — the biology of the former, on the shared deterministic state of the latter.

## Status

| Phase | Goal | State |
|-------|------|-------|
| **1. ALife core** | Prove evolution actually emerges (off-chain, deterministic) | ✅ **Done** |
| **2. Compute budget** | Benchmark real CU per sector tick on-chain (the go/no-go gate) | ✅ **Done** |
| **3. Randomness** | Tiered entropy: per-agent keyed slot hash + epoch-ahead VRF beacon | ✅ **Done** |
| 4. Tokenomics | Faucet-and-drain resource economy, keeper rewards, replication burns | ⏳ Next |
| 5. Player layer | Strain seeding, resource injection | — |

## Phase 2 result — the compute budget closes

The `tick` runs as real SBF bytecode (`programs/greygoo`), advancing one 16×16 = 256-cell sector stored zero-copy in a single account. Measured in LiteSVM (`bench/cu-bench`) against the **1.4M CU / transaction** ceiling:

| live agents in sector | CU / tick | CU / agent |
|---|---|---|
| 64  | 20,955 | 327 |
| 128 | 37,540 | 293 |
| 256 (full) | **77,037** | 300 |

- **~300 CU per agent**, statically bounded (one pass over a fixed 256 cells, no allocation).
- Worst-case full sector = **77K CU** → **18 full sectors fit in one transaction**, and a 256-sector (256×256) world advances fully in ~14 tick-txs — trivially within a block, and the txs touch disjoint accounts so they parallelize.
- The on-chain biology is live: a seeded full sector evolves down to a stable population over 200 consecutive on-chain ticks.

The compute budget closes with three orders of magnitude of headroom. Phase 2 was the real go/no-go, and it passes. *(With the Phase 3 entropy wiring added — the epoch-ahead beacon account + a SlotHashes read — the worst-case tick rises to ~89K CU, i.e. 15 sectors/tx: still the same conclusion.)*

## Phase 3 result — tiered entropy, with the manipulation measured

Native Solana entropy is **grindable** by the block leader; true VRF is **asynchronous** and can't seed a same-instruction mutation. Grey Goo tiers it (`sim-core::entropy`, wired in `programs/greygoo`):

- **Per-tick mutation** uses a cheap synchronous seed, but each agent's stream is keyed by its own identity — `agent_seed(beacon, sector, cell, strain, epoch)` — so agents are decorrelated.
- **The epoch beacon** is committed one tick ahead (from this slot's SlotHashes today, a **VRF value** later) and `sector_id` is bound to the sector's address so the caller can't choose it.

We didn't just assert this is safe — `crates/entropy-lab` **measures** what a leader grinding G candidate slot hashes can achieve (rare-event base rate p0 ≈ 0.49%):

| | G=16 | G=256 | G=4096 |
|---|---|---|---|
| **Targeted** (one known agent), naive **or** mixed | 7% | 71% | **100%** |
| **Targeted**, VRF | 0.49% | 0.49% | **0.49%** |
| **Broad** (# of 64 agents flipped at once), naive reuse | 4.9 | 46 | **64** |
| **Broad**, per-agent mixed | 1.5 | 2.7 | **3.8** |

Two precise conclusions fall out:
1. **Per-agent keying kills broad/population steering** — a single grind moves ~4 of 64 agents instead of all 64. This is what the shipped `sector::step` does.
2. **It does *not* protect a single targeted outcome** — grindable entropy makes any rare event forceable, and mixing doesn't change that. So per-tick mutation must stay **micro-stakes** (one gene ±8, which it is), and **nothing valuable may depend on it**. Anything macro/valuable uses the VRF beacon — the only design that holds at p0 under grinding.

## Phase 1 result — evolution is real

The biology is implemented as a deterministic, integer-only Rust engine (`crates/sim-core`) — the *same* state-transition logic intended to lift into the on-chain program — driven by an off-chain harness (`crates/sim-run`) that seeds **random** genomes and measures whether evolution emerges.

Across 3 independent seeds (4,000 epochs each):

- **Reproducible selection.** From random ~127 genes, every seed converges on the same adaptive strategy: `metabolism` 127 → ~15 (efficiency driven to the floor), `affinity` ↑, `aggression` ↑ (emergent predation).
- **Self-regulating population.** Settles at ~600–1,270 agents / 16,384 cells — no extinction, no grey-goo explosion. The finite grid bounds population with no explicit counter.
- **Diversity preserved.** Minimum per-gene Shannon entropy ~0.9 bits — strongly-selected genes narrow, but no dead monoculture.
- **Lineage sorting.** 3,000 founding strains collapse to 3–8 survivors; mean generation depth 300–900.
- **Bit-for-bit deterministic.** Identical reruns produce md5-identical output — the load-bearing property for the on-chain port.

*Caveat:* Phase 1 proves the biology closes. It does **not** prove the compute budget closes — that's Phase 2.

## Layout

```
crates/
  sim-core/   deterministic, integer-only ALife engine; no_std-capable, shared by
              the off-chain sim and the on-chain program (the `sector` module is
              the zero-copy on-chain representation)
  sim-run/    off-chain driver: seeds random genomes, runs N epochs, emits metrics + a verdict
  entropy-lab/ adversarial harness: measures how much a grinding leader can bias mutation
programs/
  greygoo/    native Solana (SBF) program: on-chain `tick` (sector::step + epoch-ahead beacon)
bench/
  cu-bench/   LiteSVM harness: runs the real .so and reports compute units per tick
```

## Run it

```sh
cargo run --release --bin sim-run -- 4000 1 2 3
#                                     ^     ^^^^^
#                                     epochs seeds
```

Prints per-epoch instruments and a final verdict, and writes per-seed CSVs to `out/`.

## Watch it evolve (browser)

The real `sim-core` engine is compiled to WebAssembly (`viz/greygoo-wasm`, ~17 KB, no wasm-bindgen) and driven by a canvas UI — so the browser runs the *actual* simulation, not a mock. Live grid (agents coloured by genome, resource peaks glowing teal) plus animated per-gene-mean bars where you can literally watch `metab` slide below the random-start line while `aff`/`aggr` climb above it.

```sh
node viz/serve.mjs      # → http://localhost:3210
```

Or just open `viz/standalone.html` — a single self-contained file with the wasm embedded (works offline, no server). Rebuild both after engine changes:

```sh
cargo build -p greygoo-wasm --target wasm32-unknown-unknown --release
cp <target>/wasm32-unknown-unknown/release/greygoo_wasm.wasm viz/greygoo.wasm
node viz/bundle.mjs
```

## Genome

Eight evolvable genes parameterise a fixed O(1) behaviour loop (no opcode interpreter, so per-agent work is statically bounded — required for the on-chain `tick`):

`metab` · `repro` · `move` · `aggr` · `mut` · `aff` · `split` · `dorm`

## Design constraints (Solana)

- **1.4M CU / transaction**, **12M CU per account-write-lock / block** → world state shards across sector accounts; the tick does bounded work.
- **Native randomness is leader-manipulable** → tiered entropy (cheap synchronous per-tick mutation + a VRF-backed beacon fulfilled one epoch ahead for macro events).
- **No reliable first-party scheduler** (Clockwork shut down 2023) → permissionless, staleness-weighted keeper rewards.

## License

Not yet chosen.
