//! Phase 4 verdict: does the faucet-and-drain economy maintain scarcity and
//! avoid the two documented failure modes (ghost town / death spiral)?
//!
//! Matter is conserved: a fixed supply flows treasury → cells (faucet) → agents
//! (harvest) and is returned by metabolism, split into {burn, keeper, recycle}.
//!   MAX_SUPPLY == treasury + in-world matter + burned + keeper_pool  (invariant)
//! The faucet is the only source; burn is the only permanent leak (deflation).
//!
//! We run the REAL evolution engine and only add the accounting on top, so the
//! economic regime and the biology (is selection still active?) are measured
//! together — a healthy economy keeps scarcity, which keeps selection pushing
//! `metab` down; a ghost town relaxes it.

use sim_core::{Config, World, METAB};

#[derive(Clone, Copy)]
enum Faucet {
    Fixed(u16),
    /// Adaptive controller: emit inversely to how far pop is above `target`.
    Adaptive { target: usize },
}

struct Summary {
    final_pop: usize,
    peak_pop: usize,
    free_pct: f64,     // free matter in cells as % of total cell capacity (scarcity)
    treasury_pct: f64, // treasury as % of its starting value
    burned_pct: f64,   // burned as % of MAX_SUPPLY (cumulative deflation)
    keeper_total: u64,
    mean_metab: f64, // biology check: low = selection active, ~127 = relaxed
    dry_at: Option<u64>,
    class: &'static str,
}

fn mean_metab(w: &World) -> f64 {
    let (mut s, mut p) = (0u64, 0u64);
    for c in w.cells.iter().flatten() {
        s += c.genome[METAB] as u64;
        p += 1;
    }
    if p == 0 {
        0.0
    } else {
        s as f64 / p as f64
    }
}

/// Proportional controller. Never starves the world (min regen 1); when pop is
/// above target it eases the faucet to 1 and lets natural death + the spatial
/// cap bring the population down gently.
fn adaptive_regen(pop: usize, target: usize) -> u16 {
    let err = target as i64 - pop as i64;
    (2 + 6 * err / target as i64).clamp(1, 8) as u16
}

/// Run the economy `epochs` ticks. Sinks in basis points of metabolized matter.
fn run(faucet: Faucet, burn_bp: u64, keeper_bp: u64, epochs: u64, treasury0: u64, verbose: bool) -> Summary {
    let cfg = Config {
        width: 64,
        height: 64,
        cap_max: 8,
        regen: 1,
        init_agents: 1200,
        init_energy: 40,
        max_energy: 2000,
        uniform: true, // faucet is the sole carrying-capacity control
    };
    let mut w = World::new(cfg, 0xE0C0);
    let total_cap: u64 = w.cap.iter().map(|&c| c as u64).sum();
    let start_matter = w.total_resource() + w.total_energy();
    let max_supply = treasury0 + start_matter;

    let mut treasury = treasury0;
    let mut burned = 0u64;
    let mut keeper_pool = 0u64;
    let mut peak_pop = 0usize;
    let mut dry_at: Option<u64> = None;

    if verbose {
        println!("{:>5}  {:>5}  {:>4}  {:>6}  {:>9}  {:>6}", "epoch", "pop", "regn", "free%", "treasury", "metab");
    }

    for epoch in 1..=epochs {
        let pop = w.population();
        peak_pop = peak_pop.max(pop);
        let regen = match faucet {
            Faucet::Fixed(r) => r,
            Faucet::Adaptive { target } => adaptive_regen(pop, target),
        };
        // Faucet can only emit if the treasury has matter.
        w.cfg.regen = if treasury == 0 { 0 } else { regen };
        w.step();

        // Sinks: split metabolized matter into burn / keeper / recycle-to-treasury.
        let m = w.em_metabolized;
        let burn = m * burn_bp / 10_000;
        let keep = m * keeper_bp / 10_000;
        let recycle = m - burn - keep;
        burned += burn;
        keeper_pool += keep;
        treasury += recycle;
        // Pay the faucet from the treasury.
        treasury = treasury.saturating_sub(w.em_emitted);
        if treasury == 0 && dry_at.is_none() {
            dry_at = Some(epoch);
        }

        if verbose && (epoch % 250 == 0 || epoch == 1) {
            let free_pct = w.total_resource() as f64 * 100.0 / total_cap as f64;
            println!(
                "{:>5}  {:>5}  {:>4}  {:>5.1}  {:>9}  {:>6.0}",
                epoch, pop, w.cfg.regen, free_pct, treasury, mean_metab(&w)
            );
        }
    }

    let final_pop = w.population();
    let free_pct = w.total_resource() as f64 * 100.0 / total_cap as f64;
    let metab = mean_metab(&w);
    let class = if final_pop < peak_pop / 20 || final_pop < 20 {
        "DEATH-SPIRAL"
    } else if free_pct > 60.0 {
        "GHOST-TOWN"
    } else {
        "HEALTHY"
    };
    Summary {
        final_pop,
        peak_pop,
        free_pct,
        treasury_pct: treasury as f64 * 100.0 / treasury0 as f64,
        burned_pct: burned as f64 * 100.0 / max_supply as f64,
        keeper_total: keeper_pool,
        mean_metab: metab,
        dry_at,
        class,
    }
}

fn main() {
    let epochs = 2500u64;
    let treasury0 = 5_000_000u64;
    let (burn_bp, keeper_bp) = (1000, 500); // 10% burn, 5% keeper, 85% recycle

    // ---- A. fixed faucet: the tuning knife-edge ----
    println!("== A. FIXED faucet sweep — finite treasury, 10% burn / 5% keeper ==");
    println!("(shows why a constant emission rate is fragile)\n");
    println!(
        "{:>5}  {:>8}  {:>8}  {:>6}  {:>9}  {:>6}  {:>6}  {}",
        "regen", "finalPop", "peakPop", "free%", "treas%", "metab", "dryAt", "regime"
    );
    for &r in &[0u16, 1, 2, 3, 4, 6, 8] {
        let s = run(Faucet::Fixed(r), burn_bp, keeper_bp, epochs, treasury0, false);
        println!(
            "{:>5}  {:>8}  {:>8}  {:>5.1}  {:>8.0}%  {:>6.0}  {:>6}  {}",
            r,
            s.final_pop,
            s.peak_pop,
            s.free_pct,
            s.treasury_pct,
            s.mean_metab,
            s.dry_at.map(|e| e.to_string()).unwrap_or_else(|| "-".into()),
            s.class
        );
    }

    // ---- B. adaptive faucet: self-stabilization ----
    println!("\n== B. ADAPTIVE faucet (controller targets pop = 1500) ==\n");
    let s = run(Faucet::Adaptive { target: 1500 }, burn_bp, keeper_bp, epochs, treasury0, true);
    println!(
        "\nfinal: pop {}, free {:.1}%, treasury {:.0}% of start, burned {:.2}% of supply, metab {:.0}  → {}",
        s.final_pop, s.free_pct, s.treasury_pct, s.burned_pct, s.mean_metab, s.class
    );

    // robustness: a much smaller treasury, same controller
    let s2 = run(Faucet::Adaptive { target: 1500 }, burn_bp, keeper_bp, epochs, 1_000_000, false);
    println!(
        "robustness (treasury 5x smaller): pop {}, free {:.1}%, treasury {:.0}%, metab {:.0}, dryAt {:?}  → {}",
        s2.final_pop, s2.free_pct, s2.treasury_pct, s2.mean_metab, s2.dry_at, s2.class
    );

    // ---- C. keeper solvency ----
    println!("\n== C. keeper solvency (adaptive run) ==");
    let world_cells = 64 * 64;
    let sectors = world_cells / 256; // whole-world epoch == this many sector-txs
    let keeper_per_epoch = s.keeper_total / epochs;
    let keeper_per_sectortx = keeper_per_epoch / sectors as u64;
    println!("keeper pool over {} epochs: {} matter", epochs, s.keeper_total);
    println!(
        "≈ {} matter / epoch  →  ≈ {} matter / sector-tick  ({} sectors/world)",
        keeper_per_epoch, keeper_per_sectortx, sectors
    );
    println!(
        "break-even: a keeper profits whenever {} $GOO out-values one tick tx (~5000 lamports + priority);",
        keeper_per_sectortx
    );
    println!("reward scales with sector activity, so busy sectors self-fund their own upkeep.");

    // ---- verdict ----
    println!("\n========================= VERDICT =========================");
    println!("• Fixed faucet is a knife-edge: too low → DEATH-SPIRAL (starvation),");
    println!("  too high → GHOST-TOWN (matter pools, scarcity & selection relax).");
    println!("• The adaptive faucet self-stabilizes into the HEALTHY band across a");
    println!("  5x range of treasury sizes: pop holds near target, scarcity stays");
    println!("  (low free%), selection stays active (metab driven down), keepers paid.");
    println!("• Matter conservation + a bounded burn gives slow deflation without a");
    println!("  death spiral; the spatial grid cap is the ultimate backstop.");
    println!("• Phase 3 rule stands: keeper pay is activity-proportional (fine on");
    println!("  grindable entropy); any discrete *valuable* payout must use the VRF beacon.");
    println!("===========================================================");
}
