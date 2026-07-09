//! Tiered on-chain entropy for genome mutation.
//!
//! The research constraint (Neodyme, ORAO): raw slot-hash / blockhash entropy is
//! **grindable** by the block leader, and true VRF is **asynchronous** so it
//! cannot deliver same-instruction randomness. Grey Goo resolves this by tiering:
//!
//! * **Per-tick mutation** (micro-stakes: one gene flip on one agent) uses a
//!   cheap synchronous seed. Each agent's stream is *keyed by its own immutable
//!   identity* — [`agent_seed`] mixes the tick beacon with sector/cell/strain/
//!   epoch. This decorrelates agents: a leader grinding the shared beacon cannot
//!   push the whole population the same direction at once (see the entropy-lab
//!   measurements). It does **not** by itself hide a single *known* target — see
//!   the module docs / `entropy-lab` — which is why targeted, high-value events
//!   must not depend on grindable entropy.
//!
//! * **The epoch beacon** (macro-stakes: colours a whole tick/epoch) is intended
//!   to come from a **VRF fulfilled one epoch ahead** (committed before anyone
//!   knows which agents it will touch). [`beacon_next`] models the chaining; the
//!   on-chain program feeds it slot-hash entropy today and a VRF value later.
//!
//! All functions are integer-only and `no_std`.

/// 64-bit avalanche mix of two words (splitmix64 finaliser over `a ^ (b·φ)`).
#[inline]
pub fn mix(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Per-agent mutation seed, keyed by the agent's immutable identity so agents
/// are decorrelated under a shared beacon.
#[inline]
pub fn agent_seed(beacon: u64, sector_id: u64, cell: u64, strain: u64, epoch: u64) -> u64 {
    let mut h = mix(beacon, sector_id);
    h = mix(h, cell);
    h = mix(h, strain);
    mix(h, epoch)
}

/// Chain the epoch beacon: commit the value that will seed the *next* tick from
/// the previous beacon and this slot's entropy (slot hash today, VRF later).
#[inline]
pub fn beacon_next(prev_beacon: u64, slot_entropy: u64) -> u64 {
    mix(prev_beacon, slot_entropy)
}
