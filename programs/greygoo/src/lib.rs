//! Grey Goo on-chain `tick` (native Solana program) with tiered entropy.
//!
//! One instruction advances a single sector one step. Randomness follows the
//! Phase 3 design:
//!   * This tick is seeded by the **beacon committed on the previous tick**
//!     (epoch-ahead), stored in a small world account.
//!   * Inside `sector::step`, each agent's mutation stream is keyed by the
//!     sector id + cell + strain + epoch, so grinding the beacon can't steer the
//!     whole sector at once.
//!   * `sector_id` is derived on-chain from the sector account's address (not
//!     caller-supplied), closing that grinding vector.
//!   * The next beacon is committed from this slot's SlotHashes entropy — a
//!     placeholder for a VRF value, which a leader cannot grind at all.
//!
//! Accounts: 0 = sector (writable, SECTOR_CELLS*32 bytes)
//!           1 = world beacon (writable, >=16 bytes: beacon u64 | epoch u64)
//!           2 = SlotHashes sysvar (readonly)
//! Instruction data: regen u16 (LE).

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

entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
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

    // sector_id is bound to the sector's address — not caller-chosen.
    let kb = sector.key.to_bytes();
    let sector_id = u64::from_le_bytes(kb[0..8].try_into().unwrap());

    // Read the beacon committed on the previous tick, and the epoch counter.
    let (beacon, epoch) = {
        let wd = world.try_borrow_data()?;
        if wd.len() < 16 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        (
            u64::from_le_bytes(wd[0..8].try_into().unwrap()),
            u64::from_le_bytes(wd[8..16].try_into().unwrap()),
        )
    };

    // Slot entropy for the *next* beacon: first SlotHashes entry's hash.
    // Layout: [u64 count][ (u64 slot, [u8;32] hash) ... ], most-recent first.
    let slot_entropy = {
        let sd = slot_hashes_ai.try_borrow_data()?;
        if sd.len() >= 48 {
            u64::from_le_bytes(sd[16..24].try_into().unwrap())
        } else {
            0 // sysvar not yet populated (e.g. genesis slot)
        }
    };

    // Advance the sector with the previously-committed beacon.
    {
        let mut bytes = sector.try_borrow_mut_data()?;
        let cells: &mut [Cell] = bytemuck::try_cast_slice_mut(&mut bytes[..])
            .map_err(|_| ProgramError::InvalidAccountData)?;
        if cells.len() < SECTOR_CELLS {
            return Err(ProgramError::AccountDataTooSmall);
        }
        step(&mut cells[..SECTOR_CELLS], regen, MAX_ENERGY, beacon, sector_id, epoch);
    }

    // Commit the beacon for the next tick (epoch-ahead) and bump the epoch.
    {
        let mut wd = world.try_borrow_mut_data()?;
        let nb = beacon_next(beacon, slot_entropy);
        wd[0..8].copy_from_slice(&nb.to_le_bytes());
        wd[8..16].copy_from_slice(&epoch.wrapping_add(1).to_le_bytes());
    }

    Ok(())
}
