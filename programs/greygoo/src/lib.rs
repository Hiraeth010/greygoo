//! Grey Goo on-chain program: the `tick`, the token economy, and the player
//! actions — all over sector + world accounts.
//!
//! Matter is tracked as conserved u64 counters in the world account (promoting
//! them to a real SPL `$GOO` mint is a mechanical follow-up):
//!   world = beacon u64 | epoch u64 | treasury u64 | burned u64 | keeper u64  (40 bytes)
//!
//! Instructions (opcode = data[0]):
//!   0x00 TICK   [regen u16]                              accts: sector(w) world(w) slothashes(r)
//!   0x01 SEED   [cell u16, genome[8], energy u16, strain u32]  accts: sector(w) world(w)
//!   0x02 INJECT [cell u16, amount u16]                   accts: sector(w) world(w)

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::slot_hashes,
};

use sim_core::entropy::beacon_next;
use sim_core::sector::{step, Cell, SECTOR_CELLS};

const MAX_ENERGY: i32 = 2000;
// Metabolism sink split (percent): 85 recycle / 10 burn / 5 keeper.
const BURN_PCT: u64 = 10;
const KEEPER_PCT: u64 = 5;

entrypoint!(process_instruction);

pub fn process_instruction(_pid: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let (&op, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match op {
        0x00 => tick(accounts, rest),
        0x01 => seed(accounts, rest),
        0x02 => inject(accounts, rest),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// ---- world account helpers ----
fn rd(w: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(w[o..o + 8].try_into().unwrap())
}
fn wr(w: &mut [u8], o: usize, v: u64) {
    w[o..o + 8].copy_from_slice(&v.to_le_bytes());
}
const BEACON: usize = 0;
const EPOCH: usize = 8;
const TREASURY: usize = 16;
const BURNED: usize = 24;
const KEEPER: usize = 32;

fn sector_cells<'a>(bytes: &'a mut [u8]) -> Result<&'a mut [Cell], ProgramError> {
    let cells: &mut [Cell] =
        bytemuck::try_cast_slice_mut(bytes).map_err(|_| ProgramError::InvalidAccountData)?;
    if cells.len() < SECTOR_CELLS {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(cells)
}

// ---- 0x00 TICK ----
fn tick(accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let sector = next_account_info(it)?;
    let world = next_account_info(it)?;
    let slot_hashes_ai = next_account_info(it)?;
    if !sector.is_writable || !world.is_writable {
        return Err(ProgramError::InvalidArgument);
    }
    if slot_hashes_ai.key != &slot_hashes::id() {
        return Err(ProgramError::InvalidArgument);
    }
    if data.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let regen = u16::from_le_bytes([data[0], data[1]]);

    let kb = sector.key.to_bytes();
    let sector_id = u64::from_le_bytes(kb[0..8].try_into().unwrap());

    let mut wd = world.try_borrow_mut_data()?;
    if wd.len() < 40 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let (beacon, epoch, mut treasury, mut burned, mut keeper) =
        (rd(&wd, BEACON), rd(&wd, EPOCH), rd(&wd, TREASURY), rd(&wd, BURNED), rd(&wd, KEEPER));

    // Faucet is gated by the treasury: no matter, no emission.
    let regen_eff = if treasury == 0 { 0 } else { regen };

    let slot_entropy = {
        let sd = slot_hashes_ai.try_borrow_data()?;
        if sd.len() >= 48 {
            u64::from_le_bytes(sd[16..24].try_into().unwrap())
        } else {
            0
        }
    };

    let stats = {
        let mut bytes = sector.try_borrow_mut_data()?;
        let cells = sector_cells(&mut bytes)?;
        step(&mut cells[..SECTOR_CELLS], regen_eff, MAX_ENERGY, beacon, sector_id, epoch)
    };

    // Split the metabolism sink: burn / keeper / recycle-to-treasury.
    let m = stats.metabolized;
    let burn = m * BURN_PCT / 100;
    let keep = m * KEEPER_PCT / 100;
    let recycle = m - burn - keep;
    treasury = treasury.saturating_add(recycle).saturating_sub(stats.emitted);
    burned = burned.saturating_add(burn);
    keeper = keeper.saturating_add(keep);

    wr(&mut wd, BEACON, beacon_next(beacon, slot_entropy));
    wr(&mut wd, EPOCH, epoch.wrapping_add(1));
    wr(&mut wd, TREASURY, treasury);
    wr(&mut wd, BURNED, burned);
    wr(&mut wd, KEEPER, keeper);
    Ok(())
}

// ---- 0x01 SEED: inject a designed agent, paying its energy from the treasury ----
fn seed(accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let sector = next_account_info(it)?;
    let world = next_account_info(it)?;
    if !sector.is_writable || !world.is_writable {
        return Err(ProgramError::InvalidArgument);
    }
    if data.len() < 16 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let cell = u16::from_le_bytes([data[0], data[1]]) as usize;
    let genome: [u8; 8] = data[2..10].try_into().unwrap();
    let energy = u16::from_le_bytes([data[10], data[11]]) as i32;
    let strain = u32::from_le_bytes(data[12..16].try_into().unwrap());
    if cell >= SECTOR_CELLS {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut wd = world.try_borrow_mut_data()?;
    let mut treasury = rd(&wd, TREASURY);
    if treasury < energy as u64 {
        return Err(ProgramError::InsufficientFunds);
    }

    let mut bytes = sector.try_borrow_mut_data()?;
    let cells = sector_cells(&mut bytes)?;
    if cells[cell].alive != 0 {
        return Err(ProgramError::InvalidArgument); // cell occupied
    }
    cells[cell].genome = genome;
    cells[cell].energy = energy;
    cells[cell].age = 0;
    cells[cell].gen = 0;
    cells[cell].strain = strain;
    cells[cell].alive = 1;

    treasury -= energy as u64; // matter → biomass (still conserved)
    wr(&mut wd, TREASURY, treasury);
    Ok(())
}

// ---- 0x02 INJECT: buy resource into a cell from the treasury ----
fn inject(accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let sector = next_account_info(it)?;
    let world = next_account_info(it)?;
    if !sector.is_writable || !world.is_writable {
        return Err(ProgramError::InvalidArgument);
    }
    if data.len() < 4 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let cell = u16::from_le_bytes([data[0], data[1]]) as usize;
    let amount = u16::from_le_bytes([data[2], data[3]]) as u64;
    if cell >= SECTOR_CELLS {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut wd = world.try_borrow_mut_data()?;
    let mut treasury = rd(&wd, TREASURY);

    let mut bytes = sector.try_borrow_mut_data()?;
    let cells = sector_cells(&mut bytes)?;
    let headroom = (cells[cell].cap.saturating_sub(cells[cell].resource)) as u64;
    let added = amount.min(headroom).min(treasury);
    cells[cell].resource += added as u16;

    treasury -= added;
    wr(&mut wd, TREASURY, treasury);
    Ok(())
}
