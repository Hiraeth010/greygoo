# Grey Goo

A fully on-chain, closed-system **evolutionary artificial-life simulation** for Solana. Nanotech-themed, but there is no real-world data and no oracles — the chain *is* the world. Autonomous agents ("nanobots") live on a toroidal grid, consume a scarce regenerating resource, replicate with mutated genomes, compete, and evolve. The world is advanced by a permissionless `tick`.

The design rests on two lines of established prior art: **Avida / Tierra** show that self-replicating programs competing for a scarce resource in a closed, deterministic system produce real selection, speciation, and adaptive radiation; **Dark Forest / MUD / MagicBlock** show that fully-on-chain game state is viable. Grey Goo joins the two — the biology of the former, on the shared deterministic state of the latter.

## Status

| Phase | Goal | State |
|-------|------|-------|
| **1. ALife core** | Prove evolution actually emerges (off-chain, deterministic) | ✅ **Done** |
| 2. Compute budget | Benchmark real CU per sector tick on-chain (the go/no-go gate) | ⏳ Next |
| 3. Randomness | Tiered entropy: state-mixed slot hash + epoch-ahead VRF beacon | — |
| 4. Tokenomics | Faucet-and-drain resource economy, keeper rewards, replication burns | — |
| 5. Player layer | Strain seeding, resource injection | — |

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
  sim-core/   deterministic, integer-only, dependency-free ALife engine (chain-portable)
  sim-run/    off-chain driver: seeds random genomes, runs N epochs, emits metrics + a verdict
```

## Run it

```sh
cargo run --release --bin sim-run -- 4000 1 2 3
#                                     ^     ^^^^^
#                                     epochs seeds
```

Prints per-epoch instruments and a final verdict, and writes per-seed CSVs to `out/`.

## Genome

Eight evolvable genes parameterise a fixed O(1) behaviour loop (no opcode interpreter, so per-agent work is statically bounded — required for the on-chain `tick`):

`metab` · `repro` · `move` · `aggr` · `mut` · `aff` · `split` · `dorm`

## Design constraints (Solana)

- **1.4M CU / transaction**, **12M CU per account-write-lock / block** → world state shards across sector accounts; the tick does bounded work.
- **Native randomness is leader-manipulable** → tiered entropy (cheap synchronous per-tick mutation + a VRF-backed beacon fulfilled one epoch ahead for macro events).
- **No reliable first-party scheduler** (Clockwork shut down 2023) → permissionless, staleness-weighted keeper rewards.

## License

Not yet chosen.
