//! Phase 3 verdict: measure how much a grinding block leader can bias mutation.
//!
//! Threat model: the leader producing the block containing a `tick` can grind
//! the slot hash — try up to G candidate values (by reordering / withholding /
//! stuffing transactions) and keep the one that best serves them. We measure the
//! achievable bias for three entropy designs and two attack objectives, using
//! the *real* `sim_core::mutate` and `agent_seed`.
//!
//! Designs
//!   NAIVE : reuse one grindable value as THE seed for every agent this tick.
//!   MIXED : per-agent keyed seed = agent_seed(beacon, sector, cell, strain, epoch)
//!           (what the shipped `sector::step` does).
//!   VRF   : the beacon is a value the leader cannot choose (G effectively 1).
//!
//! Objectives
//!   TARGETED : force a rare event on ONE known agent (base rate p0 ≈ 0.5%).
//!   BROAD    : force the event on MANY agents at once (population steering).

use sim_core::{entropy::agent_seed, mutate, Genome, Rng, AGGR};

const THRESH: u8 = 248; // "rare event": child aggression gene lands at the top

/// Child's aggression gene after one reproduction seeded by `seed`.
#[inline]
fn child_aggr(parent: &Genome, seed: u64) -> u8 {
    let mut r = Rng::new(seed);
    mutate(*parent, &mut r)[AGGR]
}

/// A parent poised so the event needs a single lucky +8 mutation (rare-ish).
fn poised_parent(mut_gene: u8) -> Genome {
    let mut g = [127u8; 8];
    g[AGGR] = 240;
    g[4] = mut_gene; // MUT gene index = 4
    g
}

fn main() {
    let mut rng = Rng::new(0xA11CE);
    let parent = poised_parent(127);

    // ---- base rate p0 (honest randomness) ----
    let sample = 2_000_000u64;
    let mut hits = 0u64;
    for _ in 0..sample {
        if child_aggr(&parent, rng.next_u64()) >= THRESH {
            hits += 1;
        }
    }
    let p0 = hits as f64 / sample as f64;
    println!("Rare-event base rate p0 = {:.4}%  (honest, un-grindable)\n", p0 * 100.0);

    // =========================================================================
    // OBJECTIVE 1 — TARGETED: force the event on one known agent.
    // =========================================================================
    println!("== TARGETED attack: force the rare event on ONE known agent ==");
    println!("P(leader forces the event) — grinding G candidate slot hashes:\n");
    println!("{:>8}  {:>10}  {:>10}  {:>10}", "G", "NAIVE", "MIXED", "VRF");
    let trials = 40_000u64;
    let (sector, cell, strain, epoch) = (0x1111u64, 42u64, 7u64, 99u64);
    for &g in &[1u64, 16, 64, 256, 1024, 4096] {
        // NAIVE: seed = grindable value directly.
        let naive = force_prob(trials, g, &mut rng, |v| child_aggr(&parent, v) >= THRESH);
        // MIXED: seed = per-agent keyed; attacker still knows the target's id,
        // so it's still a deterministic function of the grindable value.
        let mixed = force_prob(trials, g, &mut rng, |v| {
            child_aggr(&parent, agent_seed(v, sector, cell, strain, epoch)) >= THRESH
        });
        // VRF: leader cannot choose the value → effective G = 1 → base rate.
        let vrf = p0;
        println!(
            "{:>8}  {:>9.2}%  {:>9.2}%  {:>9.2}%",
            g,
            naive * 100.0,
            mixed * 100.0,
            vrf * 100.0
        );
    }
    println!("\n  → Grindable entropy makes ANY single valuable event forceable, and");
    println!("    per-agent mixing does NOT help a KNOWN target. Only VRF holds at p0.");
    println!("    Lesson: never gate a valuable outcome on grindable per-tick entropy.\n");

    // =========================================================================
    // OBJECTIVE 2 — BROAD: force the event on MANY agents in one grind.
    // =========================================================================
    let n = 64usize;
    println!("== BROAD attack: force the event on as many of N={} agents as possible ==", n);
    println!("Expected # agents hit simultaneously with the best of G grinds:\n");
    println!("{:>8}  {:>12}  {:>12}  {:>12}", "G", "NAIVE", "MIXED", "VRF(honest)");
    let btrials = 4_000u64;
    for &g in &[1u64, 16, 256, 4096] {
        let naive = broad_hits(btrials, g, n, &mut rng, |v, _i| {
            // NAIVE reuses ONE value for every agent → all-or-nothing, correlated.
            child_aggr(&parent, v) >= THRESH
        });
        let mixed = broad_hits(btrials, g, n, &mut rng, |v, i| {
            // MIXED keys each agent independently → decorrelated.
            child_aggr(&parent, agent_seed(v, sector, i as u64, i as u64, epoch)) >= THRESH
        });
        // VRF honest baseline: one un-chosen value, decorrelated.
        let vrf = broad_hits(btrials, 1, n, &mut rng, |v, i| {
            child_aggr(&parent, agent_seed(v, sector, i as u64, i as u64, epoch)) >= THRESH
        });
        println!(
            "{:>8}  {:>10.2}  {:>10.2}  {:>10.2}",
            g, naive, mixed, vrf
        );
    }
    println!("\n  → NAIVE reuse lets the leader flip the WHOLE population at once (all {n}).", n = n);
    println!("    MIXED decorrelates agents: even 4096 grinds move only a handful.");
    println!("    VRF stays at the honest baseline.\n");

    // ---- verdict ----
    println!("========================= VERDICT =========================");
    println!("The tiered design is validated, with one hard rule made precise:");
    println!("  1. Per-agent keyed entropy (shipped in sector::step) neutralises");
    println!("     BROAD/population steering — a single grind can't move the field.");
    println!("  2. It does NOT protect a single targeted outcome; grindable entropy");
    println!("     makes any rare event forceable. So per-tick mutation must stay");
    println!("     MICRO-stakes (one gene ±8) — which it is — and nothing valuable");
    println!("     (rewards, jackpots, ladder wins) may depend on it.");
    println!("  3. Anything macro/valuable must use the VRF beacon (epoch-ahead),");
    println!("     the only design that holds at the honest base rate under grinding.");
    println!("===========================================================");
}

/// P(at least one of G grinds triggers `hit`), averaged over `trials` situations.
fn force_prob<F: Fn(u64) -> bool>(trials: u64, g: u64, rng: &mut Rng, hit: F) -> f64 {
    let mut wins = 0u64;
    for _ in 0..trials {
        let mut ok = false;
        for _ in 0..g {
            if hit(rng.next_u64()) {
                ok = true;
                // keep drawing to not bias the stream length across G values
            }
        }
        if ok {
            wins += 1;
        }
    }
    wins as f64 / trials as f64
}

/// Expected number of the N agents hit, taking the best of G grinds each trial.
fn broad_hits<F: Fn(u64, usize) -> bool>(
    trials: u64,
    g: u64,
    n: usize,
    rng: &mut Rng,
    hit: F,
) -> f64 {
    let mut sum_best = 0u64;
    for _ in 0..trials {
        let mut best = 0usize;
        for _ in 0..g {
            let v = rng.next_u64();
            let mut c = 0usize;
            for i in 0..n {
                if hit(v, i) {
                    c += 1;
                }
            }
            if c > best {
                best = c;
            }
        }
        sum_best += best as u64;
    }
    sum_best as f64 / trials as f64
}
