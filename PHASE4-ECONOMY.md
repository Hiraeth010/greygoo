# Phase 4 — Token Economy

The goal: give the grey-goo sim **economic teeth** — a fixed-supply resource that
agents genuinely compete for, an incentive that keeps the permissionless `tick`
running, and tuning that avoids the two failure modes the deep research flagged
(ghost town / death spiral). Everything here is validated by `crates/econ-lab`,
which wraps the *real* evolution engine with the accounting and measures the
regimes rather than asserting them.

## 1. Conserved matter

There is one fungible quantity, **matter** (token `$GOO`), with a fixed maximum
supply. It is never minted or destroyed except by an explicit burn. At all times:

```
MAX_SUPPLY  ==  treasury  +  in-world matter  +  burned  +  keeper_pool
in-world matter  ==  Σ cell resource (free)  +  Σ agent energy (bound)
```

- **Treasury** — un-emitted matter, held in a program PDA.
- **Free** — matter sitting in cells as the harvestable substrate.
- **Bound** — matter locked in living biomass (agent energy).
- **Burned** — permanently removed (the only leak; drives slow deflation).
- **Keeper pool** — matter earmarked to pay whoever advances the sim.

## 2. Faucet (the only source)

Each epoch the treasury emits matter into cells (resource regrowth). This is the
**sole** source, so total emission caps the world's carrying capacity. Emission
is **adaptive** — a proportional controller that targets a population band:

```
regen = clamp(2 + 6·(target − pop)/target, 1, 8)
```

Crucially it never drops below 1 (never starves the world); when the population
runs hot it simply eases off and lets natural death + the spatial-grid cap bring
it down. The knife-edge of a *constant* faucet (measured below) is why this
feedback is not optional.

## 3. Sinks (drains)

Metabolism is the primary matter flow out of biomass; each unit metabolized is
split three ways (basis points, tunable):

| split | share | destination | purpose |
|---|---|---|---|
| **recycle** | 85% | → treasury | closes the loop so the faucet is sustainable |
| **burn** | 10% | → burned | permanent leak → slow deflation → long-run scarcity |
| **keeper** | 5% | → keeper pool | funds the permissionless tick |

Recycling is what lets a finite treasury sustain the economy indefinitely: matter
cycles treasury → cells → agents → treasury, with the burn as the only leak. The
spatial grid cap remains the ultimate backstop on population regardless of the
economy.

## 4. Keeper incentive (post-Clockwork)

Clockwork is dead (Phase 0), so the `tick` must pay for itself. The keeper pool
is funded by the 5% metabolism cut and paid to whoever submits a sector tick,
**weighted by staleness** (longest-unticked sector pays most) so no sector goes
dark. Because the cut scales with a sector's metabolic activity, **busy sectors
self-fund their own upkeep**. Measured: ≈4 matter per sector-tick in a healthy
run — a keeper profits whenever that out-values one tick tx (~5000 lamports +
priority).

**Phase 3 constraint:** keeper pay is a smooth, activity-proportional flow, so it
is safe on cheap grindable entropy. Any *discrete, valuable* payout (a jackpot, a
ladder win, a rare-trait bounty) must instead be gated on the **VRF beacon** — the
only entropy a leader cannot grind (see Phase 3 / `entropy-lab`).

## 5. Player layer ($GREY, tradeable)

- **Seed a strain** — pay to inject a designed genome into a cell (funds the
  keeper pool). The only "pay to play" surface; the sim never needs it to run.
- **Inject resource** — buy matter from the treasury into a sector.

Neither can mint matter beyond supply; both are priced by the market.

## 6. Measured regimes (`econ-lab`, uniform habitat so the faucet is the control)

**A. A constant faucet is a knife-edge:**

| regen | final pop | free % | metab | regime |
|---|---|---|---|---|
| 0 | 0 | — | — | **death spiral** (starvation) |
| 1–6 | 1.1k–2.2k | 18–42% | 12–18 | **healthy** |
| 8 | 1.3k | 73% | 22 | **ghost town** (matter pools, selection relaxes) |

Note the biology–economy coupling: in the healthy band scarcity keeps `metab`
selected **down** (~12–18); in the ghost town, matter pools, competition
evaporates and selection **relaxes** (`metab` drifts back up to ~22).

**B. The adaptive faucet self-stabilizes** at the target population with scarcity
intact (free ~30%, `metab` ~15), and stays **healthy across a 5× range of
treasury sizes** — the recycling loop sustains it even after the treasury buffer
draws down. Cumulative burn ~6.6% over 2500 epochs = slow deflation, no spiral.

## 7. Failure modes → mitigations

| Failure (research) | Mitigation here |
|---|---|
| Ghost town (weak sinks, matter pools) | adaptive faucet eases emission; burn keeps matter scarce |
| Death spiral (runaway inflation / faucet dies) | min-regen floor + recycling loop; treasury can't over-emit (conserved) |
| Population explosion | fixed spatial grid cap (hard backstop) |
| Stale sectors (no Clockwork) | staleness-weighted keeper reward |
| Entropy manipulation of payouts | valuable events gated on the VRF beacon |

## 8. On-chain shape (next build step)

- `treasury`, `keeper_pool`, `burned` counters live in the world/beacon account
  alongside the epoch beacon; the tick already touches it.
- The tick returns `em_metabolized` / `em_emitted` (already tracked in the engine)
  and applies the split in-program.
- Keeper reward paid to the tick's fee payer from `keeper_pool`.
- `$GOO` as an SPL token with the treasury PDA as mint authority under a fixed cap
  (or pre-minted supply held by the treasury).
