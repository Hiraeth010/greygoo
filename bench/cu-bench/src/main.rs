//! Phase-2 go/no-go: run the real SBF `tick` in LiteSVM and read compute units.
//!
//! Two experiments:
//!   A. Occupancy sweep — one tick over sectors seeded with K live agents, to see
//!      CU vs agent count and extract CU-per-agent.
//!   B. Evolution trace — 200 consecutive ticks on one sector, recording the
//!      max CU actually consumed as the population lives/dies/reproduces.
//!
//! Verdict math: how many agents fit under the 1.4M CU/transaction ceiling, and
//! how many full sectors that implies per block.

use bytemuck::Zeroable;
use litesvm::LiteSVM;
use sim_core::sector::{Cell, SECTOR_CELLS, SECTOR_W};
use sim_core::{Rng, GENES};
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

const MAX_TX_CU: u64 = 1_400_000; // hard per-transaction ceiling
const REGEN: u16 = 1;

/// Build a sector's raw bytes: two resource peaks + K random-genome agents.
fn build_sector(k: usize, seed: u64) -> (Vec<u8>, usize) {
    let mut cells = [Cell::zeroed(); SECTOR_CELLS];
    // Resource caps: two peaks so there is spatial structure (as in Phase 1).
    let peaks = [(5usize, 5usize), (10usize, 10usize)];
    for y in 0..SECTOR_W {
        for x in 0..SECTOR_W {
            let mut best = i32::MAX;
            for &(px, py) in peaks.iter() {
                let dx = x as i32 - px as i32;
                let dy = y as i32 - py as i32;
                best = best.min(dx * dx + dy * dy);
            }
            let cap = if best >= 36 { 1 } else { (8 - best / 5).clamp(1, 8) } as u16;
            let idx = y * SECTOR_W + x;
            cells[idx].cap = cap;
            cells[idx].resource = cap;
        }
    }
    // Place K agents at random empty cells with random genomes.
    let mut rng = Rng::new(seed);
    let mut placed = 0usize;
    let mut attempts = 0usize;
    while placed < k && attempts < k * 50 {
        attempts += 1;
        let i = (rng.next_u64() as usize) % SECTOR_CELLS;
        if cells[i].alive != 0 {
            continue;
        }
        let mut genome = [0u8; GENES];
        for g in genome.iter_mut() {
            *g = rng.byte();
        }
        cells[i].genome = genome;
        cells[i].energy = 40;
        cells[i].alive = 1;
        cells[i].strain = placed as u32;
        placed += 1;
    }
    (bytemuck::cast_slice::<Cell, u8>(&cells).to_vec(), placed)
}

fn instruction_data(seed: u64, epoch: u64) -> Vec<u8> {
    let mut d = Vec::with_capacity(18);
    d.extend_from_slice(&seed.to_le_bytes());
    d.extend_from_slice(&epoch.to_le_bytes());
    d.extend_from_slice(&REGEN.to_le_bytes());
    d
}

fn alive_count(data: &[u8]) -> usize {
    let cells: &[Cell] = bytemuck::cast_slice(&data[..SECTOR_CELLS * 32]);
    cells.iter().filter(|c| c.alive != 0).count()
}

fn main() {
    let so_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "D:/cargo-target/deploy/greygoo_program.so".to_string());

    let program_id = Pubkey::new_unique();
    let payer = Keypair::new();

    let mut svm = LiteSVM::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
    svm.add_program_from_file(program_id, &so_path)
        .expect("load .so (pass path as arg 1 if not at default)");

    let sector_size_bytes = SECTOR_CELLS * 32;
    println!(
        "program: {}\nsector: {}x{} = {} cells, {} bytes/account\n",
        so_path, SECTOR_W, SECTOR_W, SECTOR_CELLS, sector_size_bytes
    );

    // ---- Experiment A: occupancy sweep (fresh account each K) ----
    println!("== A. occupancy sweep (single tick) ==");
    println!("{:>6}  {:>7}  {:>9}  {:>9}", "agents", "CU", "CU/agent", "under1.4M");
    for &k in &[16usize, 32, 64, 96, 128, 160, 192, 224, 256] {
        let sector_pk = Pubkey::new_unique();
        let (data, placed) = build_sector(k, 0xABCD_0000 + k as u64);
        svm.set_account(
            sector_pk,
            Account {
                lamports: 1_000_000_000,
                data,
                owner: program_id,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

        let ix = Instruction {
            program_id,
            accounts: vec![AccountMeta::new(sector_pk, false)],
            data: instruction_data(0xF00D, 1),
        };
        let msg = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[&payer], msg, svm.latest_blockhash());
        match svm.send_transaction(tx) {
            Ok(meta) => {
                let cu = meta.compute_units_consumed;
                let per = if placed > 0 { cu / placed as u64 } else { 0 };
                let fits = MAX_TX_CU / cu.max(1);
                println!("{:>6}  {:>7}  {:>9}  {:>9}", placed, cu, per, format!("{}x", fits));
            }
            Err(e) => println!("{:>6}  tx FAILED: {:?}", placed, e.err),
        }
    }

    // ---- Experiment B: evolution trace (persistent account, 200 ticks) ----
    println!("\n== B. evolution trace (200 consecutive ticks, full sector) ==");
    let sector_pk = Pubkey::new_unique();
    let (data, placed) = build_sector(256, 0x1234);
    svm.set_account(
        sector_pk,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    let mut max_cu = 0u64;
    let mut sum_cu = 0u64;
    let mut min_cu = u64::MAX;
    let ticks = 200u64;
    println!("start pop = {}", placed);
    println!("{:>5}  {:>7}  {:>4}", "tick", "CU", "pop");
    for epoch in 1..=ticks {
        let ix = Instruction {
            program_id,
            accounts: vec![AccountMeta::new(sector_pk, false)],
            data: instruction_data(0x5EED, epoch),
        };
        let msg = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[&payer], msg, svm.latest_blockhash());
        let meta = svm.send_transaction(tx).expect("tick tx");
        let cu = meta.compute_units_consumed;
        max_cu = max_cu.max(cu);
        min_cu = min_cu.min(cu);
        sum_cu += cu;
        if epoch % 25 == 0 || epoch == 1 {
            let acc = svm.get_account(&sector_pk).unwrap();
            println!("{:>5}  {:>7}  {:>4}", epoch, cu, alive_count(&acc.data));
        }
    }

    // ---- Verdict ----
    let final_pop = alive_count(&svm.get_account(&sector_pk).unwrap().data);
    let mean_cu = sum_cu / ticks;
    println!("\n== VERDICT ==");
    println!("evolution-trace CU: min {} / mean {} / max {}", min_cu, mean_cu, max_cu);
    println!("final pop: {}", final_pop);
    let sectors_per_tx = MAX_TX_CU / max_cu.max(1);
    println!(
        "\nworst-case sector tick = {} CU  →  {} full sectors per 1.4M-CU transaction",
        max_cu, sectors_per_tx
    );
    println!(
        "a 256x256 world = {} sectors; at {} sectors/tx that is ~{} tick-txs to advance the whole world once",
        (256 / SECTOR_W) * (256 / SECTOR_W),
        sectors_per_tx,
        ((256 / SECTOR_W) * (256 / SECTOR_W)) as u64 / sectors_per_tx.max(1)
    );
    let verdict = max_cu < MAX_TX_CU;
    println!(
        "\n>>> a single sector tick fits in one tx: {} <<<",
        if verdict { "YES ✅" } else { "NO ❌" }
    );
}
