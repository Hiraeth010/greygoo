//! Fixed-layout, zero-copy sector — the on-chain representation of a patch of
//! world. A sector is a small toroidal sub-grid stored as a flat array of
//! [`Cell`]s inside a single Solana account. `step` advances one sector one
//! tick with **statically bounded** work (no allocation, no recursion, one pass
//! over a fixed number of cells) so its compute cost is measurable and capped —
//! exactly what the permissionless on-chain `tick` needs.
//!
//! The per-agent logic here mirrors the off-chain `World::step` that Phase 1
//! used to prove evolution emerges; both share `Rng`, the genome constants,
//! `mutate`, and the trait-mapping fns from the crate root, so the biology
//! cannot silently diverge between the prototype and the chain.

use crate::entropy::agent_seed;
use crate::{harvest_cap, metab_cost, mutate, repro_threshold, Genome, Rng, AGGR, DORM, MOVE, SPLIT};

pub const SECTOR_W: usize = 16;
pub const SECTOR_H: usize = 16;
pub const SECTOR_CELLS: usize = SECTOR_W * SECTOR_H; // 256

/// One grid cell: location data (resource/cap) stays put; agent data
/// (genome/energy/…) moves when the agent relocates. `#[repr(C)]` + `Pod` so
/// the account's raw bytes can be reinterpreted as `&mut [Cell]` with zero copy.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Cell {
    // --- location (never moves) ---
    pub resource: u16,
    pub cap: u16,
    // --- agent (moves with the organism) ---
    pub age: u16,
    pub gen: u16,
    pub strain: u32,
    pub energy: i32,
    pub genome: Genome, // [u8; 8]
    pub alive: u8,
    pub _pad: [u8; 7],
}

// Compile-time guarantee the layout is exactly 32 bytes (256 cells = 8 KiB).
const _: () = assert!(core::mem::size_of::<Cell>() == 32);

impl Cell {
    /// Copy just the agent-portion fields from `src` into `self`, marking alive.
    #[inline]
    fn become_agent(&mut self, genome: Genome, energy: i32, age: u16, gen: u16, strain: u32) {
        self.genome = genome;
        self.energy = energy;
        self.age = age;
        self.gen = gen;
        self.strain = strain;
        self.alive = 1;
    }
}

#[inline]
fn bit_get(bits: &[u64; 4], i: usize) -> bool {
    (bits[i >> 6] >> (i & 63)) & 1 != 0
}
#[inline]
fn bit_set(bits: &mut [u64; 4], i: usize) {
    bits[i >> 6] |= 1 << (i & 63);
}

#[inline]
fn neighbors(i: usize) -> [usize; 4] {
    let x = i % SECTOR_W;
    let y = i / SECTOR_W;
    let left = (x + SECTOR_W - 1) % SECTOR_W + y * SECTOR_W;
    let right = (x + 1) % SECTOR_W + y * SECTOR_W;
    let up = x + ((y + SECTOR_H - 1) % SECTOR_H) * SECTOR_W;
    let down = x + ((y + 1) % SECTOR_H) * SECTOR_W;
    [left, right, up, down]
}

/// Greenest empty neighbour (first-found tie-break → deterministic).
#[inline]
fn best_empty(cells: &[Cell], nb: &[usize; 4]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_res: i32 = -1;
    for &k in nb.iter() {
        if cells[k].alive == 0 && cells[k].resource as i32 > best_res {
            best_res = cells[k].resource as i32;
            best = Some(k);
        }
    }
    best
}

/// Matter flows and vital stats from one sector tick — consumed by the on-chain
/// economy (faucet cost + metabolism sink) and useful for telemetry.
#[derive(Clone, Copy, Default)]
pub struct StepStats {
    pub emitted: u64,     // matter the faucet added (a treasury cost)
    pub metabolized: u64, // matter metabolism consumed (the sink to split)
    pub births: u32,
    pub deaths: u32,
}

/// Advance one sector one tick. `beacon` is this tick's committed entropy
/// (on-chain: the epoch-ahead beacon; see [`crate::entropy`]); `sector_id` and
/// `epoch` key each agent's mutation stream so agents are decorrelated under a
/// shared beacon — a leader grinding `beacon` cannot steer the whole sector.
/// Returns the [`StepStats`] the economy layer meters.
pub fn step(cells: &mut [Cell], regen: u16, max_energy: i32, beacon: u64, sector_id: u64, epoch: u64) -> StepStats {
    debug_assert_eq!(cells.len(), SECTOR_CELLS);
    let mut stats = StepStats::default();

    // 1. Resource regrowth (the faucet).
    for c in cells.iter_mut() {
        if c.resource < c.cap {
            let before = c.resource;
            c.resource = (c.resource + regen).min(c.cap);
            stats.emitted += (c.resource - before) as u64;
        }
    }

    // 2. Agent updates — one fixed pass, `moved` guards agents relocated ahead.
    let mut moved = [0u64; 4];
    for i in 0..SECTOR_CELLS {
        if cells[i].alive == 0 || bit_get(&moved, i) {
            continue;
        }
        bit_set(&mut moved, i);

        // Read agent into locals.
        let genome = cells[i].genome;
        let mut energy = cells[i].energy;
        let age = cells[i].age.saturating_add(1);
        let gen = cells[i].gen;
        let strain = cells[i].strain;

        // Per-agent entropy stream, keyed by this agent's immutable identity.
        let mut arng = Rng::new(agent_seed(beacon, sector_id, i as u64, strain as u64, epoch));

        // -- harvest current cell --
        let h = cells[i].resource.min(harvest_cap(&genome));
        cells[i].resource -= h;
        energy += h as i32;

        // -- metabolism (dormancy softens the cost when starving) --
        let mut cost = metab_cost(&genome);
        if h == 0 {
            let dorm = genome[DORM] as i32;
            cost = 1 + (cost - 1) * (255 - dorm) / 255;
        }
        energy -= cost;
        stats.metabolized += cost as u64;

        // -- death --
        if energy <= 0 {
            cells[i].alive = 0;
            stats.deaths += 1;
            continue;
        }
        if energy > max_energy {
            energy = max_energy;
        }

        let nb = neighbors(i);

        // -- reproduction into the greenest empty neighbour --
        if energy >= repro_threshold(&genome) {
            if let Some(k) = best_empty(cells, &nb) {
                let split = genome[SPLIT] as i32;
                let child_energy = (energy * split / 255).clamp(1, energy - 1);
                energy -= child_energy;
                let child_genome = mutate(genome, &mut arng);
                cells[k].become_agent(child_genome, child_energy, 0, gen.saturating_add(1), strain);
                bit_set(&mut moved, k);
                stats.births += 1;
            }
        }

        // -- movement / predation --
        let move_roll = arng.chance(genome[MOVE]);
        let aggr_roll = arng.chance(genome[AGGR]);
        let mut target = i;

        if move_roll {
            if let Some(k) = best_empty(cells, &nb) {
                if cells[k].resource > cells[i].resource {
                    target = k;
                }
            }
        }
        if target == i && aggr_roll {
            let mut victim: Option<usize> = None;
            let mut victim_e = energy;
            for &k in nb.iter() {
                if cells[k].alive != 0 && cells[k].energy < victim_e {
                    victim_e = cells[k].energy;
                    victim = Some(k);
                }
            }
            if let Some(k) = victim {
                let gained = cells[k].energy / 2;
                cells[k].alive = 0;
                stats.deaths += 1;
                energy = (energy + gained).min(max_energy);
                target = k;
            }
        }

        // Commit agent to its final cell (may be `i`).
        if target != i {
            cells[i].alive = 0;
        }
        cells[target].become_agent(genome, energy, age, gen, strain);
        bit_set(&mut moved, target);
    }

    stats
}
