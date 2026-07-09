// Grey Goo — devnet bridge. Renders the on-chain sector fetched straight from
// Solana, streams the program's real transaction history as a live feed, and
// lets a Phantom wallet send real tick / seed / inject transactions.
import * as web3 from 'https://esm.sh/@solana/web3.js@1.95.3?bundle';

const $ = (id) => document.getElementById(id);
const SECTOR_CELLS = 256, CELL = 32;
const OP = { 0: 'advanced the world', 1: 'seeded a strain', 2: 'injected resource', 3: 'initialised world', 4: 'initialised sector' };
const OPCLASS = { 0: 'fd-tick', 1: 'fd-seed', 2: 'fd-inject', 3: 'fd-init', 4: 'fd-init' };

let cfg, conn, PROGRAM, WORLD, SECTOR, SLOT_HASHES, PROGRAM_B58;
let wallet = null, walletPk = null, lastSector = null;
const seen = new Map();

// ---- LE encoders (match the Rust program) ----
function bytes(parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.len, 0));
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
const ixTick = () => mkIx([meta(SECTOR, true), meta(WORLD, true), meta(SLOT_HASHES, false)], bytes([{ u8: 0, len: 1 }, { u16: 4, len: 2 }]));
const ixSeed = (cell, g, energy, strain) => mkIx([meta(SECTOR, true), meta(WORLD, true)], bytes([{ u8: 1, len: 1 }, { u16: cell, len: 2 }, { raw: g, len: 8 }, { u16: energy, len: 2 }, { u32: strain, len: 4 }]));
const ixInject = (cell, amount) => mkIx([meta(SECTOR, true), meta(WORLD, true)], bytes([{ u8: 2, len: 1 }, { u16: cell, len: 2 }, { u16: amount, len: 2 }]));

// ---- base58 decode (for reading the instruction opcode) ----
const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function bs58(str) {
  const bytes = [0];
  for (const ch of str) {
    let carry = B58.indexOf(ch); if (carry < 0) return new Uint8Array();
    for (let j = 0; j < bytes.length; j++) { carry += bytes[j] * 58; bytes[j] = carry & 0xff; carry >>= 8; }
    while (carry) { bytes.push(carry & 0xff); carry >>= 8; }
  }
  for (let k = 0; k < str.length && str[k] === '1'; k++) bytes.push(0);
  return new Uint8Array(bytes.reverse());
}

// ---- on-chain state ----
const aliveAt = (d, i) => d[i * CELL + 24] !== 0;
function readWorld(d) {
  const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
  return { epoch: Number(dv.getBigUint64(8, true)), treasury: Number(dv.getBigUint64(16, true)), burned: Number(dv.getBigUint64(24, true)), keeper: Number(dv.getBigUint64(32, true)) };
}
const fmt = (n) => n.toLocaleString();
function renderSector(d) {
  const off = document.createElement('canvas'); off.width = 16; off.height = 16;
  const octx = off.getContext('2d'); const img = octx.createImageData(16, 16);
  let pop = 0;
  for (let i = 0; i < SECTOR_CELLS; i++) {
    const b = i * CELL, o = i * 4;
    if (aliveAt(d, i)) { pop++; const mt = d[b + 16], ag = d[b + 19], af = d[b + 21]; img.data[o] = af; img.data[o + 1] = 255 - mt; img.data[o + 2] = ag; img.data[o + 3] = 255; }
    else { const res = d[b] | (d[b + 1] << 8), cap = d[b + 2] | (d[b + 3] << 8); const gl = Math.min(150, 18 + (cap ? (res * 150) / cap : 0)); img.data[o] = 3; img.data[o + 1] = gl / 2 + 6; img.data[o + 2] = gl; img.data[o + 3] = 255; }
  }
  octx.putImageData(img, 0, 0);
  const cv = $('oc-grid'), ctx = cv.getContext('2d');
  ctx.imageSmoothingEnabled = false; ctx.clearRect(0, 0, cv.width, cv.height); ctx.drawImage(off, 0, 0, cv.width, cv.height);
  return pop;
}
function randomEmptyCell(d) { const e = []; for (let i = 0; i < SECTOR_CELLS; i++) if (!aliveAt(d, i)) e.push(i); return e.length ? e[(Math.random() * e.length) | 0] : 0; }

async function refresh() {
  try {
    const [s, w] = await Promise.all([conn.getAccountInfo(SECTOR), conn.getAccountInfo(WORLD)]);
    if (s) { lastSector = new Uint8Array(s.data); $('oc-pop').textContent = renderSector(lastSector); }
    if (w) { const x = readWorld(new Uint8Array(w.data)); $('oc-epoch').textContent = fmt(x.epoch); $('oc-treasury').textContent = fmt(x.treasury); $('oc-burned').textContent = fmt(x.burned); $('oc-keeper').textContent = fmt(x.keeper); }
  } catch (e) {}
}

// ---- live transaction feed ----
function timeAgo(t) {
  if (!t) return 'now';
  const s = Math.max(0, Math.floor(Date.now() / 1000 - t));
  if (s < 60) return s + 's ago';
  if (s < 3600) return Math.floor(s / 60) + 'm ago';
  if (s < 86400) return Math.floor(s / 3600) + 'h ago';
  return Math.floor(s / 86400) + 'd ago';
}
function classify(tx) {
  if (!tx) return { action: 'transaction', op: -1, actor: null };
  const msg = tx.transaction.message;
  const actor = msg.accountKeys?.[0]?.pubkey?.toBase58?.() ?? null;
  let op = -1;
  for (const ins of msg.instructions || []) {
    const pid = ins.programId?.toBase58?.() ?? ins.programId;
    if (pid === PROGRAM_B58) { if (ins.data) { const d = bs58(ins.data); if (d.length) op = d[0]; } break; }
  }
  return { action: OP[op] || 'transaction', op, actor };
}
function rowEl(e) {
  const row = document.createElement('div'); row.className = 'feed-row';
  const cls = e.err ? 'fd-err' : (OPCLASS[e.op] || 'fd-init');
  const who = e.actor ? e.actor.slice(0, 4) + '…' + e.actor.slice(-4) : '';
  row.innerHTML =
    `<span class="feed-dot ${cls}"></span>` +
    `<div class="feed-main"><a class="feed-act" href="https://explorer.solana.com/tx/${e.sig}?cluster=devnet" target="_blank" rel="noopener">${e.err ? e.action + ' · failed' : e.action} <span class="who">${who}</span></a></div>` +
    `<span class="feed-time">${timeAgo(e.time)}</span>`;
  return row;
}
async function pollFeed() {
  let sigs;
  try { sigs = await conn.getSignaturesForAddress(PROGRAM, { limit: 25 }); } catch { return; }
  const fresh = sigs.filter((s) => !seen.has(s.signature));
  if (fresh.length) {
    let parsed = [];
    try { parsed = await conn.getParsedTransactions(fresh.map((s) => s.signature), { maxSupportedTransactionVersion: 0 }); } catch {}
    fresh.forEach((s, i) => seen.set(s.signature, { sig: s.signature, err: !!s.err, time: s.blockTime, ...classify(parsed[i]) }));
    const list = $('feed-list');
    const empty = list.querySelector('.feed-empty'); if (empty) empty.remove();
    for (let i = fresh.length - 1; i >= 0; i--) list.prepend(rowEl(seen.get(fresh[i].signature)));
    while (list.children.length > 40) list.removeChild(list.lastChild);
  }
  $('feed-count').textContent = `${seen.size} seen`;
}

// ---- wallet + actions ----
async function connect() {
  const p = window.solana;
  if (!p || !p.isPhantom) { window.open('https://phantom.app/', '_blank'); return; }
  try {
    const r = await p.connect(); wallet = p; walletPk = r.publicKey;
    $('oc-addr').textContent = walletPk.toBase58();
    $('oc-connect').textContent = 'Wallet connected';
    ['oc-tick', 'oc-seed', 'oc-inject'].forEach((id) => ($(id).disabled = false));
  } catch (e) {}
}
async function sendIx(ix, btn) {
  const label = btn.querySelector('.k')?.previousSibling?.textContent || '';
  const prev = btn.innerHTML; btn.disabled = true; btn.textContent = 'signing…';
  try {
    const { blockhash } = await conn.getLatestBlockhash();
    const tx = new web3.Transaction().add(ix); tx.feePayer = walletPk; tx.recentBlockhash = blockhash;
    const signed = await wallet.signTransaction(tx);
    btn.textContent = 'confirming…';
    const sig = await conn.sendRawTransaction(signed.serialize());
    await conn.confirmTransaction(sig, 'confirmed');
    await refresh(); await pollFeed();
  } catch (e) {
    btn.textContent = (e?.message || 'failed').slice(0, 24);
    await new Promise((r) => setTimeout(r, 1600));
  }
  btn.innerHTML = prev; btn.disabled = false; void label;
}

async function init() {
  cfg = await (await fetch('devnet.json')).json();
  conn = new web3.Connection(cfg.rpc, 'confirmed');
  PROGRAM = new web3.PublicKey(cfg.programId); PROGRAM_B58 = cfg.programId;
  WORLD = new web3.PublicKey(cfg.world); SECTOR = new web3.PublicKey(cfg.sector);
  SLOT_HASHES = web3.SYSVAR_SLOT_HASHES_PUBKEY;

  const ex = (id, kind) => `https://explorer.solana.com/${kind}/${id}?cluster=devnet`;
  $('lnk-program').href = ex(cfg.programId, 'address');
  $('lnk-sector').href = ex(cfg.sector, 'address');
  $('lnk-world').href = ex(cfg.world, 'address');
  $('lnk-goo').href = ex(cfg.goo, 'address');

  $('oc-connect').onclick = connect;
  $('oc-tick').onclick = (e) => sendIx(ixTick(), e.currentTarget);
  $('oc-seed').onclick = (e) => { const cell = lastSector ? randomEmptyCell(lastSector) : 0; const g = new Uint8Array(8); crypto.getRandomValues(g); sendIx(ixSeed(cell, g, 120, (Math.random() * 0xffffffff) >>> 0), e.currentTarget); };
  $('oc-inject').onclick = (e) => { const cell = lastSector ? randomEmptyCell(lastSector) : 0; sendIx(ixInject(cell, 8), e.currentTarget); };

  await refresh(); await pollFeed();
  setInterval(refresh, 6000);
  setInterval(pollFeed, 7000);
}
init().catch((e) => { const l = $('feed-list'); if (l) l.innerHTML = `<div class="feed-empty" style="color:#ff8f8f">bridge failed: ${e.message}</div>`; });
