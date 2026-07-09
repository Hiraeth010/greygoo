// Grey Goo · devnet bootstrap.
// Creates + initializes the world and sector accounts on devnet, then runs the
// real tick / seed / inject instructions against them. Saves the account
// addresses to site/devnet.json so the browser can target the same world.
import {
  Connection, Keypair, PublicKey, SystemProgram, Transaction, TransactionInstruction,
  SYSVAR_SLOT_HASHES_PUBKEY, sendAndConfirmTransaction,
} from '@solana/web3.js';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';

const RPC = 'https://api.devnet.solana.com';
const PROGRAM_ID = new PublicKey('6mLbTSrKTU1xbrkpe2q2zpSr4xTy6DUJ2nnLiAb1CYDh');
const SECTOR_BYTES = 256 * 32;
const WORLD_BYTES = 40;
const DIR = fileURLToPath(new URL('.', import.meta.url));

const conn = new Connection(RPC, 'confirmed');
const kpPath = process.env.SOLANA_KEYPAIR || `${homedir()}/.config/solana/id.json`;
const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(kpPath))));

// ---- byte encoders (must match the Rust program exactly) ----
const u8 = (n) => Buffer.from([n & 0xff]);
const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n >>> 0); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const cat = (...xs) => Buffer.concat(xs.map((x) => (Buffer.isBuffer(x) ? x : Buffer.from(x))));

const meta = (pk, writable) => ({ pubkey: pk, isSigner: false, isWritable: writable });
const ix = (keys, data) => new TransactionInstruction({ programId: PROGRAM_ID, keys, data });

const ixInitWorld = (world, treasury, beacon) => ix([meta(world, true)], cat(u8(0x03), u64(treasury), u64(beacon)));
const ixInitSector = (sector, cap, seed, n) => ix([meta(sector, true)], cat(u8(0x04), u8(cap), u64(seed), u16(n)));
const ixTick = (sector, world, regen) =>
  ix([meta(sector, true), meta(world, true), meta(SYSVAR_SLOT_HASHES_PUBKEY, false)], cat(u8(0x00), u16(regen)));
const ixSeed = (sector, world, cell, genome, energy, strain) =>
  ix([meta(sector, true), meta(world, true)], cat(u8(0x01), u16(cell), Buffer.from(genome), u16(energy), u32(strain)));
const ixInject = (sector, world, cell, amount) =>
  ix([meta(sector, true), meta(world, true)], cat(u8(0x02), u16(cell), u16(amount)));

const send = async (instructions, signers = [payer]) => {
  const tx = new Transaction().add(...instructions);
  return sendAndConfirmTransaction(conn, tx, signers, { commitment: 'confirmed' });
};
const explorer = (id, kind = 'tx') => `https://explorer.solana.com/${kind}/${id}?cluster=devnet`;
const aliveCount = (data) => {
  let n = 0;
  for (let i = 0; i < 256; i++) if (data[i * 32 + 24] !== 0) n++; // alive flag at cell offset 24
  return n;
};
const firstEmptyCell = (data) => {
  for (let i = 0; i < 256; i++) if (data[i * 32 + 24] === 0) return i;
  return 0;
};

async function main() {
  console.log('payer:', payer.publicKey.toBase58());
  console.log('balance:', (await conn.getBalance(payer.publicKey)) / 1e9, 'SOL\n');

  const statePath = `${DIR}state.json`;
  let world, sector, fresh;
  if (existsSync(statePath)) {
    const s = JSON.parse(readFileSync(statePath));
    world = Keypair.fromSecretKey(Uint8Array.from(s.worldSecret));
    sector = Keypair.fromSecretKey(Uint8Array.from(s.sectorSecret));
    fresh = false;
    console.log('reusing existing world/sector from state.json');
  } else {
    world = Keypair.generate();
    sector = Keypair.generate();
    fresh = true;
    const rentW = await conn.getMinimumBalanceForRentExemption(WORLD_BYTES);
    const rentS = await conn.getMinimumBalanceForRentExemption(SECTOR_BYTES);
    console.log('creating accounts (rent:', (rentW + rentS) / 1e9, 'SOL) …');
    const sig = await send(
      [
        SystemProgram.createAccount({ fromPubkey: payer.publicKey, newAccountPubkey: world.publicKey, lamports: rentW, space: WORLD_BYTES, programId: PROGRAM_ID }),
        SystemProgram.createAccount({ fromPubkey: payer.publicKey, newAccountPubkey: sector.publicKey, lamports: rentS, space: SECTOR_BYTES, programId: PROGRAM_ID }),
      ],
      [payer, world, sector]
    );
    console.log('  created:', explorer(sig));
    writeFileSync(statePath, JSON.stringify({
      world: world.publicKey.toBase58(), sector: sector.publicKey.toBase58(),
      worldSecret: [...world.secretKey], sectorSecret: [...sector.secretKey],
    }, null, 2));
  }
  console.log('world :', world.publicKey.toBase58());
  console.log('sector:', sector.publicKey.toBase58(), '\n');

  if (fresh) {
    console.log('init_world + init_sector …');
    await send([ixInitWorld(world.publicKey, 4_000_000, 0xABCD)]);
    await send([ixInitSector(sector.publicKey, 8, Date.now() & 0xffffffff, 200)]);
  }

  const readSector = async () => (await conn.getAccountInfo(sector.publicKey)).data;
  console.log('seeded population:', aliveCount(await readSector()));

  console.log('\nadvancing 5 ticks on-chain …');
  for (let i = 0; i < 5; i++) {
    const sig = await send([ixTick(sector.publicKey, world.publicKey, 4)]);
    process.stdout.write(`  tick ${i + 1}: ${sig.slice(0, 12)}…  `);
  }
  console.log('\npopulation now:', aliveCount(await readSector()));

  console.log('\nseeding a designed strain + injecting resource …');
  const empty = firstEmptyCell(await readSector());
  await send([ixSeed(sector.publicKey, world.publicKey, empty, [10, 127, 40, 220, 127, 220, 160, 127], 120, 4242)]);
  await send([ixInject(sector.publicKey, world.publicKey, empty, 8)]);
  console.log(`seeded strain into empty cell ${empty}; population:`, aliveCount(await readSector()));

  // publish account addresses for the browser
  const cfg = { rpc: RPC, programId: PROGRAM_ID.toBase58(), world: world.publicKey.toBase58(), sector: sector.publicKey.toBase58() };
  writeFileSync(`${DIR}../site/devnet.json`, JSON.stringify(cfg, null, 2));
  console.log('\nwrote site/devnet.json');
  console.log('\nexplorer:');
  console.log('  program:', explorer(PROGRAM_ID.toBase58(), 'address'));
  console.log('  sector :', explorer(sector.publicKey.toBase58(), 'address'));
  console.log('  world  :', explorer(world.publicKey.toBase58(), 'address'));
}

main().catch((e) => { console.error(e); process.exit(1); });
