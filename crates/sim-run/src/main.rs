//! Grey Goo Phase-1 harness: seed random genomes, run the deterministic core for
//! N epochs across several seeds, and print instruments that reveal whether real
//! evolution is happening — not drift, not collapse.
//!
//! Verdict criteria:
//!   1. Population self-regulates (no extinction, no explosion to the grid cap).
//!   2. Gene means shift DIRECTIONALLY from the random ~127.5 init toward adaptive
//!      values, REPRODUCIBLY in the same direction across independent seeds.
//!   3. Diversity persists (per-gene Shannon entropy stays well above 0 — i.e. no
//!      instant monoculture), while still showing structure (below the ~4-bit max).

use sim_core::{Config, Metrics, World, GENES, GENE_NAMES};
use std::fs::{self, File};
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let epochs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let seeds: Vec<u64> = {
        let rest: Vec<u64> = args.filter_map(|s| s.parse().ok()).collect();
        if rest.is_empty() {
            vec![1, 2, 3]
        } else {
            rest
        }
    };

    fs::create_dir_all("out").ok();

    let mut initials: Vec<[f64; GENES]> = Vec::new();
    let mut finals: Vec<Metrics> = Vec::new();

    for &seed in &seeds {
        println!("\n=== seed {seed} : {epochs} epochs ===");
        let cfg = Config::default();
        let n = cfg.width * cfg.height;
        let mut world = World::new(cfg, seed);

        let init = world.metrics();
        initials.push(init.gene_mean);
        println!(
            "epoch     pop  births  deaths  m.age  m.energy  strains  top%   {}",
            GENE_NAMES.join("    ")
        );
        print_row(&init, n);

        let mut csv = File::create(format!("out/seed_{seed}.csv")).unwrap();
        write_csv_header(&mut csv);
        write_csv_row(&mut csv, &init);

        let report_every = (epochs / 12).max(1);
        for _ in 0..epochs {
            world.step();
            if world.epoch % report_every == 0 {
                let m = world.metrics();
                print_row(&m, n);
                write_csv_row(&mut csv, &m);
            }
        }
        let fin = world.metrics();
        finals.push(fin);
        println!("wrote out/seed_{seed}.csv");
    }

    // ---- cross-seed verdict ----
    println!("\n===================== VERDICT =====================");
    let init_mean = mean_of(&initials); // ~127.5 per gene by construction

    println!("\nGene means — random init vs evolved (per seed):");
    println!(
        "{:<7} {:>7}   {}",
        "gene",
        "init",
        seeds
            .iter()
            .map(|s| format!("seed{s:>2}"))
            .collect::<Vec<_>>()
            .join("  ")
    );
    let mut directional = 0usize;
    for g in 0..GENES {
        let deltas: Vec<f64> = finals.iter().map(|m| m.gene_mean[g] - init_mean[g]).collect();
        let all_up = deltas.iter().all(|d| *d > 6.0);
        let all_down = deltas.iter().all(|d| *d < -6.0);
        let reproducible = all_up || all_down;
        if reproducible {
            directional += 1;
        }
        let cells: String = finals
            .iter()
            .map(|m| format!("{:>6.0}", m.gene_mean[g]))
            .collect::<Vec<_>>()
            .join("  ");
        let flag = if reproducible {
            if all_up { "  ↑ selected up" } else { "  ↓ selected down" }
        } else {
            ""
        };
        println!("{:<7} {:>7.0}   {}{}", GENE_NAMES[g], init_mean[g], cells, flag);
    }

    println!("\nDiversity (final per-gene Shannon entropy, bits; 0=monoculture, ~4=uniform):");
    for (idx, m) in finals.iter().enumerate() {
        let ent: String = (0..GENES)
            .map(|g| format!("{:>4.1}", m.gene_entropy[g]))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  seed {:>2}: {}", seeds[idx], ent);
    }

    println!("\nPopulation & lineage (final):");
    for (idx, m) in finals.iter().enumerate() {
        println!(
            "  seed {:>2}: pop={:<6} strains_alive={:<5} top_strain={:>4.0}%  mean_gen={:>6.0}  mean_age={:>5.0}",
            seeds[idx],
            m.population,
            m.strains_alive,
            m.top_strain_share * 100.0,
            m.mean_gen,
            m.mean_age
        );
    }

    // ---- automated pass/fail ----
    let extinct = finals.iter().any(|m| m.population == 0);
    let exploded = finals.iter().any(|m| m.population as f64 > 0.95 * (128.0 * 128.0));
    let min_ent: f64 = finals
        .iter()
        .flat_map(|m| m.gene_entropy.iter().copied())
        .fold(f64::INFINITY, f64::min);
    let diversity_ok = min_ent > 0.5;
    let selection_ok = directional >= 3;

    println!("\n---------------------------------------------------");
    println!(
        "self-regulating population : {}",
        yn(!extinct && !exploded)
    );
    println!(
        "reproducible selection     : {}  ({directional}/{GENES} genes moved same way across all seeds)",
        yn(selection_ok)
    );
    println!("diversity preserved        : {}  (min gene entropy {:.2} bits)", yn(diversity_ok), min_ent);
    let real = !extinct && !exploded && selection_ok && diversity_ok;
    println!("\n>>> EVOLUTION IS {} <<<", if real { "REAL ✅" } else { "NOT DEMONSTRATED ❌" });
    println!("===================================================");
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "NO" }
}

fn mean_of(rows: &[[f64; GENES]]) -> [f64; GENES] {
    let mut out = [0f64; GENES];
    for r in rows {
        for g in 0..GENES {
            out[g] += r[g];
        }
    }
    for g in 0..GENES {
        out[g] /= rows.len() as f64;
    }
    out
}

fn print_row(m: &Metrics, _n: usize) {
    let genes: String = (0..GENES)
        .map(|g| format!("{:>5.0}", m.gene_mean[g]))
        .collect::<Vec<_>>()
        .join("");
    println!(
        "{:>6}  {:>5}  {:>6}  {:>6}  {:>5.0}  {:>7.0}  {:>6}  {:>4.0}  {}",
        m.epoch,
        m.population,
        m.births,
        m.deaths,
        m.mean_age,
        m.mean_energy,
        m.strains_alive,
        m.top_strain_share * 100.0,
        genes
    );
}

fn write_csv_header(f: &mut File) {
    let mut cols = vec![
        "epoch".to_string(),
        "pop".into(),
        "births".into(),
        "deaths".into(),
        "mean_age".into(),
        "mean_energy".into(),
        "mean_gen".into(),
        "strains".into(),
        "top_share".into(),
    ];
    for g in GENE_NAMES.iter() {
        cols.push(format!("mean_{g}"));
    }
    for g in GENE_NAMES.iter() {
        cols.push(format!("ent_{g}"));
    }
    writeln!(f, "{}", cols.join(",")).ok();
}

fn write_csv_row(f: &mut File, m: &Metrics) {
    let mut cols = vec![
        m.epoch.to_string(),
        m.population.to_string(),
        m.births.to_string(),
        m.deaths.to_string(),
        format!("{:.2}", m.mean_age),
        format!("{:.2}", m.mean_energy),
        format!("{:.2}", m.mean_gen),
        m.strains_alive.to_string(),
        format!("{:.4}", m.top_strain_share),
    ];
    for g in 0..GENES {
        cols.push(format!("{:.2}", m.gene_mean[g]));
    }
    for g in 0..GENES {
        cols.push(format!("{:.3}", m.gene_entropy[g]));
    }
    writeln!(f, "{}", cols.join(",")).ok();
}
