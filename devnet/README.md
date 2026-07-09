# Grey Goo · devnet

Deploys the program to Solana devnet and bootstraps the shared world + sector
the site talks to.

## Addresses (devnet)

- Program `6mLbTSrKTU1xbrkpe2q2zpSr4xTy6DUJ2nnLiAb1CYDh`
- Sector  `H8obp85KqoKVZkkJTpzCPtUwDZxQb5NanG6i9eLGTZQH`
- World   `DbhomRJyicnvgiLJmky7YzR9Y9ByhsZsCJDuqEQp6fCE`
- $GOO    `AoWLSzrK2M1rctNKZjTHWGXJRfYzt9UdD8gTvqAiwzRk` (fixed supply, mint authority disabled)

## Program instructions (opcode = first data byte)

| op | name | data (LE) | accounts |
|----|------|-----------|----------|
| `0x00` | tick | `regen u16` | sector(w), world(w), SlotHashes(r) |
| `0x01` | seed_strain | `cell u16, genome[8], energy u16, strain u32` | sector(w), world(w) |
| `0x02` | inject_resource | `cell u16, amount u16` | sector(w), world(w) |
| `0x03` | init_world | `treasury u64, beacon u64` | world(w) |
| `0x04` | init_sector | `cap u8, seed u64, n_agents u16` | sector(w) |

## Run

```sh
# deploy the program (from repo root, after `cargo build-sbf` in programs/greygoo)
solana program deploy <target>/deploy/greygoo_program.so \
  --program-id <target>/deploy/greygoo_program-keypair.json --url devnet

# create + init the world/sector, run a few real txs, write site/devnet.json
npm install
node bootstrap.mjs
```

`bootstrap.mjs` reuses `state.json` (git-ignored — holds the world/sector
account keypairs) on re-runs, so it's idempotent. It signs with your local
`solana` devnet keypair (`~/.config/solana/id.json`).
