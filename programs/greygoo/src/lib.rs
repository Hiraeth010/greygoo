//! Grey Goo on-chain `tick` (native Solana program).
//!
//! One instruction: advance a single sector one step. The sector lives as the
//! raw bytes of one writable account, reinterpreted zero-copy as `&mut [Cell]`.
//! All the biology is `sim_core::sector::step` — the exact code proven off-chain
//! in Phase 1 — so this program is a thin, measurable shell around it. Its whole
//! purpose in Phase 2 is to let us measure real SBF compute units per tick.
//!
//! Instruction data (18 bytes, little-endian): seed u64 | epoch u64 | regen u16.
//! Account 0: the sector (writable), data == SECTOR_CELLS * size_of::<Cell>().

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey,
};

use sim_core::sector::{step, Cell, SECTOR_CELLS};

/// Matches `sim_core::Config` default; the resource/agent scale the sim uses.
const MAX_ENERGY: i32 = 2000;

entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let sector = accounts.first().ok_or(ProgramError::NotEnoughAccountKeys)?;
    if !sector.is_writable {
        return Err(ProgramError::InvalidArgument);
    }
    if data.len() < 18 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let seed = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let epoch = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let regen = u16::from_le_bytes(data[16..18].try_into().unwrap());

    let mut bytes = sector.try_borrow_mut_data()?;
    let cells: &mut [Cell] =
        bytemuck::try_cast_slice_mut(&mut bytes[..]).map_err(|_| ProgramError::InvalidAccountData)?;
    if cells.len() < SECTOR_CELLS {
        return Err(ProgramError::AccountDataTooSmall);
    }

    step(&mut cells[..SECTOR_CELLS], regen, MAX_ENERGY, seed, epoch);
    Ok(())
}
