// Grey Goo — devnet bridge. Renders the on-chain sector fetched straight from
// Solana, and lets a Phantom wallet send real tick / seed / inject transactions.
// The instruction byte layouts here must match the Rust program exactly.
import * as web3 from 'https://esm.sh/@solana/web3.js@1.95.3?bundle';

const $ = (id) => document.getElementById(id);
const SECTOR_CELLS = 256;
const CELL = 32;

let cfg, conn, PROGRAM, WORLD, SECTOR, SLOT_HASHES;
let wallet = null, walletPk = null;

// ---- little-endian encoders → Uint8Array (match the program) ----
function bytes(parts) {
  const total = parts.reduce((n, p) => n + p.len, 0);
  const out = new Uint8Array(total);
  const dv = new DataView(out.buffer);
  let o = 0;
  for (const p of parts) {
    if (p.u8 !== undefined) dv.setUint8(o, p.u8);
    else if (p.u16 !== undefined) dv.setUint16(o, p.u16, true);
    else if (p.u32 !== undefined) dv.setUint32(o, p.u32 >>> 0, true);
    else if (p.raw) out.set(p.raw, o);
    o += p.len;
  }
  return out;
}
const meta = (pk, w) => ({ pubkey: pk, isSigner: false, isWritable: w });
const mkIx = (keys, data) => new web3.TransactionInstruction({ programId: PROGRAM, keys, data });

const ixTick = () => mkIx(
  [meta(SECTOR, true), meta(WORLD, true), meta(SLOT_HASHES, false)],
  bytes([{ u8: 0x00, len: 1 }, { u16: 4, len: 2 }]) // regen 4
);
const ixSeed = (cell, genome, energy, strain) => mkIx(
  [meta(SECTOR, true), meta(WORLD, true)],
  bytes([{ u8: 0x01, len: 1 }, { u16: cell, len: 2 }, { raw: genome, len: 8 }, { u16: energy, len: 2 }, { u32: strain, len: 4 }])
);
const ixInject = (cell, amount) => mkIx(
  [meta(SECTOR, true), meta(WORLD, true)],
  bytes([{ u8: 0x02, len: 1 }, { u16: cell, len: 2 }, { u16: amount, len: 2 }])
);

// ---- read on-chain state ----
const aliveAt = (d, i) => d[i * CELL + 24] !== 0;
function readWorld(d) {
  const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
  return {
    epoch: Number(dv.getBigUint64(8, true)),
    treasury: Number(dv.getBigUint64(16, true)),
    burned: Number(dv.getBigUint64(24, true)),
    keeper: Number(dv.getBigUint64(32, true)),
  };
}
const fmt = (n) => n.toLocaleString();

function renderSector(d) {
  const W = 16, H = 16;
  const off = document.createElement('canvas'); off.width = W; off.height = H;
  const octx = off.getContext('2d');
  const img = octx.createImageData(W, H);
  let pop = 0;
  for (let i = 0; i < SECTOR_CELLS; i++) {
    const b = i * CELL, o = i * 4;
    if (aliveAt(d, i)) {
      pop++;
      const metab = d[b + 16], aggr = d[b + 16 + 3], aff = d[b + 16 + 5];
      img.data[o] = aff; img.data[o + 1] = 255 - metab; img.data[o + 2] = aggr; img.data[o + 3] = 255;
    } else {
      const res = d[b] | (d[b + 1] << 8), cap = d[b + 2] | (d[b + 3] << 8);
      const glow = Math.min(150, 18 + (cap ? (res * 150) / cap : 0));
      img.data[o] = 3; img.data[o + 1] = glow / 2 + 6; img.data[o + 2] = glow; img.data[o + 3] = 255;
    }
  }
  octx.putImageData(img, 0, 0);
  const cv = $('oc-grid'), ctx = cv.getContext('2d');
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, cv.width, cv.height);
  ctx.drawImage(off, 0, 0, cv.width, cv.height);
  return pop;
}

function randomEmptyCell(d) {
  const empties = [];
  for (let i = 0; i < SECTOR_CELLS; i++) if (!aliveAt(d, i)) empties.push(i);
  return empties.length ? empties[(Math.random() * empties.length) | 0] : 0;
}

let lastSector = null;
async function refresh() {
  try {
    const [sAcc, wAcc] = await Promise.all([conn.getAccountInfo(SECTOR), conn.getAccountInfo(WORLD)]);
    if (sAcc) { lastSector = new Uint8Array(sAcc.data); $('oc-pop').textContent = renderSector(lastSector); }
    if (wAcc) {
      const w = readWorld(new Uint8Array(wAcc.data));
      $('oc-epoch').textContent = fmt(w.epoch);
      $('oc-treasury').textContent = fmt(w.treasury);
      $('oc-burned').textContent = fmt(w.burned);
      $('oc-keeper').textContent = fmt(w.keeper);
    }
  } catch (e) { /* transient RPC hiccup; next tick retries */ }
}

// ---- tx log ----
function logRow(label) {
  const row = document.createElement('div');
  row.className = 'pending';
  row.textContent = `↑ ${label} — sending…`;
  $('oc-log').prepend(row);
  return {
    done: (sig) => { row.className = ''; row.innerHTML = `✓ ${label} — <a href="https://explorer.solana.com/tx/${sig}?cluster=devnet" target="_blank" rel="noopener">${sig.slice(0, 8)}…${sig.slice(-6)} ↗</a>`; },
    fail: (msg) => { row.className = 'err'; row.textContent = `✕ ${label} — ${msg}`; },
  };
}

async function sendIx(ix, label) {
  const row = logRow(label);
  try {
    const { blockhash } = await conn.getLatestBlockhash();
    const tx = new web3.Transaction().add(ix);
    tx.feePayer = walletPk;
    tx.recentBlockhash = blockhash;
    const signed = await wallet.signTransaction(tx); // wallet signs; we send to devnet
    const sig = await conn.sendRawTransaction(signed.serialize());
    await conn.confirmTransaction(sig, 'confirmed');
    row.done(sig);
    await refresh();
  } catch (e) {
    row.fail((e && e.message ? e.message : String(e)).slice(0, 80));
  }
}

// ---- wallet ----
async function connect() {
  const p = window.solana;
  if (!p || !p.isPhantom) {
    window.open('https://phantom.app/', '_blank');
    return;
  }
  try {
    const r = await p.connect();
    wallet = p; walletPk = r.publicKey;
    $('oc-addr').textContent = walletPk.toBase58();
    $('oc-connect').textContent = 'Wallet connected';
    ['oc-tick', 'oc-seed', 'oc-inject'].forEach((id) => ($(id).disabled = false));
  } catch (e) { /* user rejected */ }
}

async function init() {
  cfg = await (await fetch('devnet.json')).json();
  conn = new web3.Connection(cfg.rpc, 'confirmed');
  PROGRAM = new web3.PublicKey(cfg.programId);
  WORLD = new web3.PublicKey(cfg.world);
  SECTOR = new web3.PublicKey(cfg.sector);
  SLOT_HASHES = web3.SYSVAR_SLOT_HASHES_PUBKEY;

  const ex = (id, kind) => `https://explorer.solana.com/${kind}/${id}?cluster=devnet`;
  $('lnk-program').href = ex(cfg.programId, 'address');
  $('lnk-sector').href = ex(cfg.sector, 'address');
  $('lnk-world').href = ex(cfg.world, 'address');
  $('lnk-goo').href = ex(cfg.goo, 'address');

  $('oc-connect').onclick = connect;
  $('oc-tick').onclick = () => sendIx(ixTick(), 'tick');
  $('oc-seed').onclick = () => {
    const cell = lastSector ? randomEmptyCell(lastSector) : 0;
    const g = new Uint8Array(8); crypto.getRandomValues(g);
    sendIx(ixSeed(cell, g, 120, (Math.random() * 0xffffffff) >>> 0), `seed → cell ${cell}`);
  };
  $('oc-inject').onclick = () => {
    const cell = lastSector ? randomEmptyCell(lastSector) : 0;
    sendIx(ixInject(cell, 8), `inject → cell ${cell}`);
  };

  await refresh();
  setInterval(refresh, 6000);
}

init().catch((e) => {
  const log = $('oc-log');
  if (log) log.innerHTML = `<div class="err">on-chain bridge failed to load: ${e.message}</div>`;
});
