//! Grey Goo — deterministic, integer-only artificial-life core.
//!
//! This crate is the biology. It is deliberately dependency-free, integer-only,
//! and free of wall-clock / OS randomness so that the *exact same* state-transition
//! logic can later be lifted into a Solana program (BPF) where determinism across
//! validators is mandatory. The off-chain `sim-run` driver exists only to answer
//! one question cheaply: **does evolution actually emerge?**
//!
//! Model: a toroidal sugarscape. Each cell holds at most one agent and a
//! regenerating resource. Agents harvest, pay a metabolic cost, die at zero
//! energy, and reproduce (with mutation) into empty neighbours. Genomes are 8
//! evolvable genes that parameterise a fixed O(1) behaviour loop — no opcode
//! interpreter, so per-agent work is statically bounded (the property the
//! on-chain `tick` will need).

// ---------------------------------------------------------------------------
// Deterministic PRNG (splitmix64). Stands in for on-chain hash-derived entropy.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed ^ 0x243F_6A88_85A3_08D3 }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform byte 0..=255.
    #[inline]
    pub fn byte(&mut self) -> u8 {
        (self.next_u64() >> 24) as u8
    }

    /// True with probability `threshold/256`.
    #[inline]
    pub fn chance(&mut self, threshold: u8) -> bool {
        self.byte() < threshold
    }

    /// Inclusive integer in [lo, hi].
    #[inline]
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i32
    }
}

// ---------------------------------------------------------------------------
// Genome
// ---------------------------------------------------------------------------

pub const GENES: usize = 8;

// Gene indices.
pub const METAB: usize = 0; // metabolism: energy burned per tick
pub const REPRO: usize = 1; // reproduction energy threshold
pub const MOVE: usize = 2; // mobility tendency
pub const AGGR: usize = 3; // aggression / predation
pub const MUT: usize = 4; // self-controlled mutation rate
pub const AFF: usize = 5; // resource affinity (harvest efficiency)
pub const SPLIT: usize = 6; // offspring energy split
pub const DORM: usize = 7; // dormancy (starvation resistance)

pub const GENE_NAMES: [&str; GENES] =
    ["metab", "repro", "move", "aggr", "mut", "aff", "split", "dorm"];

/// Minimum per-gene mutation probability (out of 256) so exploration never
/// freezes even if the MUT gene is selected to zero.
const MUT_FLOOR: u8 = 6;

pub type Genome = [u8; GENES];

#[inline]
fn mutate(mut g: Genome, rng: &mut Rng) -> Genome {
    let thr = MUT_FLOOR.saturating_add(g[MUT] / 8); // ~2.3%..14% per gene
    for gene in g.iter_mut() {
        if rng.chance(thr) {
            let delta = rng.range_i32(-8, 8);
            *gene = (*gene as i32 + delta).clamp(0, 255) as u8;
        }
    }
    g
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Agent {
    pub genome: Genome,
    pub energy: i32,
    pub age: u16,
    pub gen: u16,   // lineage depth (generation number)
    pub strain: u32, // founding-ancestor id
    acted: u64,     // last epoch this agent acted (async-update guard)
}

// Derived traits (kept as free fns so the mapping is explicit and portable).
#[inline]
fn metab_cost(g: &Genome) -> i32 {
    1 + (g[METAB] as i32) / 32 // 1..8
}
#[inline]
fn repro_threshold(g: &Genome) -> i32 {
    40 + g[REPRO] as i32 // 40..295
}
#[inline]
fn harvest_cap(g: &Genome) -> u16 {
    1 + (g[AFF] as u16) / 32 // 1..8
}

// ---------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------

pub struct Config {
    pub width: usize,
    pub height: usize,
    pub cap_max: u16,
    pub regen: u16,
    pub init_agents: usize,
    pub init_energy: i32,
    pub max_energy: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            width: 128,
            height: 128,
            cap_max: 8,
            regen: 1,
            init_agents: 3000,
            init_energy: 40,
            max_energy: 2000,
        }
    }
}

pub struct World {
    pub cfg: Config,
    pub cells: Vec<Option<Agent>>,
    pub resource: Vec<u16>,
    pub cap: Vec<u16>,
    pub epoch: u64,
    pub rng: Rng,
    pub births: u64,
    pub deaths: u64,
}

impl World {
    pub fn new(cfg: Config, seed: u64) -> Self {
        let n = cfg.width * cfg.height;
        let mut rng = Rng::new(seed);

        // Two resource peaks (classic sugarscape) → spatial heterogeneity, so
        // different regions can favour different strategies (niches → radiation).
        let peaks = [
            (cfg.width / 3, cfg.height / 3),
            (2 * cfg.width / 3, 2 * cfg.height / 3),
        ];
        let mut cap = vec![0u16; n];
        for y in 0..cfg.height {
            for x in 0..cfg.width {
                // Toroidal-aware nearest-peak falloff, integer only.
                let mut best = i32::MAX;
                for &(px, py) in peaks.iter() {
                    let dx = tor_delta(x, px, cfg.width);
                    let dy = tor_delta(y, py, cfg.height);
                    let d2 = dx * dx + dy * dy;
                    if d2 < best {
                        best = d2;
                    }
                }
                // cap falls off with distance; radius ~ width/4.
                let radius2 = ((cfg.width / 4) * (cfg.width / 4)) as i32;
                let c = if best >= radius2 {
                    0
                } else {
                    // linear falloff cap_max .. 1 across the radius
                    let frac = (radius2 - best) * (cfg.cap_max as i32) / radius2;
                    frac.clamp(0, cfg.cap_max as i32)
                };
                cap[y * cfg.width + x] = c as u16;
            }
        }
        let resource = cap.clone();

        let mut cells: Vec<Option<Agent>> = (0..n).map(|_| None).collect();

        // Seed agents with fully random genomes at random empty cells.
        let mut placed = 0u32;
        let mut attempts = 0usize;
        let target = cfg.init_agents.min(n);
        while (placed as usize) < target && attempts < target * 50 {
            attempts += 1;
            let i = (rng.next_u64() as usize) % n;
            if cells[i].is_some() {
                continue;
            }
            let mut genome = [0u8; GENES];
            for gene in genome.iter_mut() {
                *gene = rng.byte();
            }
            cells[i] = Some(Agent {
                genome,
                energy: cfg.init_energy,
                age: 0,
                gen: 0,
                strain: placed,
                acted: u64::MAX, // never acted
            });
            placed += 1;
        }

        World {
            cfg,
            cells,
            resource,
            cap,
            epoch: 0,
            rng,
            births: 0,
            deaths: 0,
        }
    }

    #[inline]
    fn neighbors(&self, i: usize) -> [usize; 4] {
        let w = self.cfg.width;
        let h = self.cfg.height;
        let x = i % w;
        let y = i / w;
        let left = (x + w - 1) % w + y * w;
        let right = (x + 1) % w + y * w;
        let up = x + ((y + h - 1) % h) * w;
        let down = x + ((y + 1) % h) * w;
        [left, right, up, down]
    }

    /// Advance the whole world one epoch.
    pub fn step(&mut self) {
        self.epoch += 1;
        let epoch = self.epoch;
        let n = self.cells.len();

        // 1. Resource regrowth.
        for i in 0..n {
            let c = self.cap[i];
            if self.resource[i] < c {
                self.resource[i] = (self.resource[i] + self.cfg.regen).min(c);
            }
        }

        // 2. Agent updates (asynchronous, fixed row-major order).
        for i in 0..n {
            // Skip empty cells and agents that already acted this epoch
            // (e.g. moved here from a lower index).
            match &self.cells[i] {
                Some(a) if a.acted == epoch => continue,
                Some(_) => {}
                None => continue,
            }
            let mut agent = self.cells[i].take().unwrap();
            agent.acted = epoch;
            agent.age = agent.age.saturating_add(1);

            // -- harvest current cell --
            let cap = harvest_cap(&agent.genome);
            let h = self.resource[i].min(cap);
            self.resource[i] -= h;
            agent.energy += h as i32;
            let harvested = h;

            // -- metabolism (dormancy softens the cost when starving) --
            let mut cost = metab_cost(&agent.genome);
            if harvested == 0 {
                let dorm = agent.genome[DORM] as i32;
                cost = 1 + (cost - 1) * (255 - dorm) / 255;
            }
            agent.energy -= cost;

            // -- death --
            if agent.energy <= 0 {
                self.deaths += 1;
                // cell already None
                continue;
            }
            if agent.energy > self.cfg.max_energy {
                agent.energy = self.cfg.max_energy;
            }

            let nb = self.neighbors(i);

            // -- reproduction into the greenest empty neighbour --
            if agent.energy >= repro_threshold(&agent.genome) {
                if let Some(k) = self.best_empty(&nb) {
                    let split = agent.genome[SPLIT] as i32;
                    let mut child_energy = agent.energy * split / 255;
                    child_energy = child_energy.clamp(1, agent.energy - 1);
                    agent.energy -= child_energy;
                    let child = Agent {
                        genome: mutate(agent.genome, &mut self.rng),
                        energy: child_energy,
                        age: 0,
                        gen: agent.gen.saturating_add(1),
                        strain: agent.strain,
                        acted: epoch,
                    };
                    self.cells[k] = Some(child);
                    self.births += 1;
                }
            }

            // -- movement / predation --
            let mut target = i;
            let move_roll = self.rng.chance(agent.genome[MOVE]);
            let aggr_roll = self.rng.chance(agent.genome[AGGR]);

            if move_roll {
                if let Some(k) = self.best_empty(&nb) {
                    if self.resource[k] > self.resource[i] {
                        target = k; // relocate to greener pasture
                    }
                }
            }
            if target == i && aggr_roll {
                // attack the weakest weaker neighbour, absorb half its energy
                let mut victim: Option<usize> = None;
                let mut victim_e = agent.energy;
                for &k in nb.iter() {
                    if let Some(o) = &self.cells[k] {
                        if o.energy < victim_e {
                            victim_e = o.energy;
                            victim = Some(k);
                        }
                    }
                }
                if let Some(k) = victim {
                    let gained = self.cells[k].take().unwrap().energy / 2;
                    self.deaths += 1;
                    agent.energy = (agent.energy + gained).min(self.cfg.max_energy);
                    target = k;
                }
            }

            self.cells[target] = Some(agent);
        }
    }

    /// Greenest empty neighbour, ties broken by first-found (deterministic).
    #[inline]
    fn best_empty(&self, nb: &[usize; 4]) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_res = -1i32;
        for &k in nb.iter() {
            if self.cells[k].is_none() && self.resource[k] as i32 > best_res {
                best_res = self.resource[k] as i32;
                best = Some(k);
            }
        }
        best
    }

    pub fn population(&self) -> usize {
        self.cells.iter().filter(|c| c.is_some()).count()
    }

    pub fn metrics(&self) -> Metrics {
        Metrics::compute(self)
    }
}

#[inline]
fn tor_delta(a: usize, b: usize, span: usize) -> i32 {
    let d = (a as i32 - b as i32).abs();
    d.min(span as i32 - d)
}

// ---------------------------------------------------------------------------
// Metrics — the instruments that tell us whether evolution is real
// ---------------------------------------------------------------------------

pub struct Metrics {
    pub epoch: u64,
    pub population: usize,
    pub gene_mean: [f64; GENES],
    pub gene_entropy: [f64; GENES], // Shannon entropy over 16 bins, in bits (0..4)
    pub mean_age: f64,
    pub mean_energy: f64,
    pub mean_gen: f64,
    pub strains_alive: usize,
    pub top_strain_share: f64,
    pub births: u64,
    pub deaths: u64,
}

impl Metrics {
    pub fn compute(w: &World) -> Metrics {
        let mut pop = 0usize;
        let mut sum_gene = [0u64; GENES];
        let mut hist = [[0u32; 16]; GENES];
        let mut sum_age = 0u64;
        let mut sum_energy = 0i64;
        let mut sum_gen = 0u64;
        // strain counts via a small map (strain ids are dense-ish but can be sparse)
        let mut strain_counts: std::collections::HashMap<u32, u32> =
            std::collections::HashMap::new();

        for c in w.cells.iter() {
            if let Some(a) = c {
                pop += 1;
                for g in 0..GENES {
                    sum_gene[g] += a.genome[g] as u64;
                    hist[g][(a.genome[g] >> 4) as usize] += 1;
                }
                sum_age += a.age as u64;
                sum_energy += a.energy as i64;
                sum_gen += a.gen as u64;
                *strain_counts.entry(a.strain).or_insert(0) += 1;
            }
        }

        let mut gene_mean = [0f64; GENES];
        let mut gene_entropy = [0f64; GENES];
        if pop > 0 {
            for g in 0..GENES {
                gene_mean[g] = sum_gene[g] as f64 / pop as f64;
                let mut ent = 0f64;
                for &count in hist[g].iter() {
                    if count > 0 {
                        let p = count as f64 / pop as f64;
                        ent -= p * p.log2();
                    }
                }
                gene_entropy[g] = ent;
            }
        }

        let top = strain_counts.values().copied().max().unwrap_or(0);
        Metrics {
            epoch: w.epoch,
            population: pop,
            gene_mean,
            gene_entropy,
            mean_age: if pop > 0 { sum_age as f64 / pop as f64 } else { 0.0 },
            mean_energy: if pop > 0 { sum_energy as f64 / pop as f64 } else { 0.0 },
            mean_gen: if pop > 0 { sum_gen as f64 / pop as f64 } else { 0.0 },
            strains_alive: strain_counts.len(),
            top_strain_share: if pop > 0 { top as f64 / pop as f64 } else { 0.0 },
            births: w.births,
            deaths: w.deaths,
        }
    }
}
