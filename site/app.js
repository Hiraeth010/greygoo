// Grey Goo — site driver. Runs the real engine (WASM) as a teeming live culture,
// mirrors its vitals in real time, and lets you recolour it by genome / lineage /
// vitality. Same deterministic code that runs on-chain.
document.documentElement.classList.add('js');

const GENES = ['metab', 'repro', 'move', 'aggr', 'mut', 'aff', 'split', 'dorm'];
const HOT = new Set(['metab', 'aff', 'aggr']);
const W = 220, H = 220, INIT_AGENTS = 16000, SCALE = 560;

const $ = (id) => document.getElementById(id);
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

// ---- gene telemetry rows (in the "what you're watching" section) ----
const rows = {};
const rowsEl = $('gene-rows');
GENES.forEach((name) => {
  const row = document.createElement('div');
  row.className = 'gene' + (HOT.has(name) ? ' hot' : '');
  row.innerHTML =
    `<span class="g-name">${name}</span>` +
    `<div class="track"><div class="fill"></div><div class="base"></div></div>` +
    `<span class="g-val">–</span>`;
  rowsEl.appendChild(row);
  rows[name] = { fill: row.querySelector('.fill'), val: row.querySelector('.g-val') };
});

// ---- scroll reveals ----
const io = new IntersectionObserver(
  (es) => es.forEach((e) => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }),
  { threshold: 0.14, rootMargin: '0px 0px -8% 0px' }
);
document.querySelectorAll('.phase, .reveal').forEach((el) => { if (reduceMotion) el.classList.add('in'); else io.observe(el); });

// ---- engine ----
const cv = $('grid'); cv.width = SCALE; cv.height = SCALE;
const ctx = cv.getContext('2d');
const sparkCv = $('spark'); const sctx = sparkCv.getContext('2d');

let wasm, app, imgData, off, offCtx;
let playing = true, speed = 5, mode = 0, frame = 0;
const popHist = [];
let lastSample = 0, lastBirths = 0, lastDeaths = 0, divRate = 0, deathRate = 0;

const u32 = (f) => f >>> 0;

function newWorld() {
  if (app) wasm.free_app(app);
  const seed = (Math.floor(performance.now() * 1000) ^ 0x9e3779b9) >>> 0;
  app = wasm.init(u32(seed), 0, W, H, INIT_AGENTS, 1); // uniform habitat → full field
  off = document.createElement('canvas'); off.width = W; off.height = H;
  offCtx = off.getContext('2d');
  imgData = offCtx.createImageData(W, H);
  popHist.length = 0;
  lastBirths = 0; lastDeaths = 0; lastSample = performance.now();
}

function drawSpark() {
  const w = sparkCv.width, h = sparkCv.height;
  sctx.clearRect(0, 0, w, h);
  if (popHist.length < 2) return;
  const max = Math.max(...popHist, 1);
  const step = w / (popHist.length - 1);
  sctx.beginPath();
  popHist.forEach((p, i) => { const x = i * step, y = h - (p / max) * (h - 4) - 2; i ? sctx.lineTo(x, y) : sctx.moveTo(x, y); });
  sctx.lineTo(w, h); sctx.lineTo(0, h); sctx.closePath();
  sctx.fillStyle = 'rgba(90,224,138,0.10)'; sctx.fill();
  sctx.beginPath();
  popHist.forEach((p, i) => { const x = i * step, y = h - (p / max) * (h - 4) - 2; i ? sctx.lineTo(x, y) : sctx.moveTo(x, y); });
  sctx.strokeStyle = 'rgba(120,232,168,0.9)'; sctx.lineWidth = 1.5; sctx.stroke();
}

function draw() {
  const ptr = wasm.render(app, mode);
  imgData.data.set(new Uint8Array(wasm.memory.buffer, ptr, W * H * 4));
  offCtx.putImageData(imgData, 0, 0);

  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, SCALE, SCALE);
  ctx.drawImage(off, 0, 0, SCALE, SCALE);
  if (!reduceMotion) { // bloom: soft additive pass for a luminous, alive field
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.globalAlpha = 0.55; ctx.filter = 'blur(4px)';
    ctx.drawImage(off, 0, 0, SCALE, SCALE);
    ctx.restore();
  }

  if (frame % 3 === 0) {
    const pop = wasm.population(app);
    $('v-pop').textContent = pop.toLocaleString();
    $('v-epoch').textContent = wasm.epoch(app).toLocaleString();
    $('v-gen').textContent = wasm.mean_gen(app).toFixed(0);
    $('v-strain').textContent = wasm.strains(app).toLocaleString();
    const metab = wasm.gene_mean(app, 0);
    const mv = $('v-metab'); mv.textContent = metab.toFixed(0);
    mv.className = 'v-v mono ' + (metab < 120 ? 'up' : ''); // low metab = selection working (good)
    GENES.forEach((name, i) => {
      const m = wasm.gene_mean(app, i);
      rows[name].fill.style.width = (m / 255 * 100).toFixed(1) + '%';
      rows[name].val.textContent = m.toFixed(0);
    });
  }

  const now = performance.now();
  if (now - lastSample > 850) {
    const b = wasm.births(app), d = wasm.deaths(app);
    const dt = (now - lastSample) / 1000;
    divRate = Math.max(0, Math.round((b - lastBirths) / dt));
    deathRate = Math.max(0, Math.round((d - lastDeaths) / dt));
    lastBirths = b; lastDeaths = d; lastSample = now;
    $('v-div').textContent = divRate.toLocaleString();
    $('v-death').textContent = deathRate.toLocaleString();
    popHist.push(wasm.population(app)); if (popHist.length > 90) popHist.shift();
    drawSpark();
  }
}

function loop() { if (playing) wasm.step(app, speed); draw(); frame++; requestAnimationFrame(loop); }

$('play').onclick = () => { playing = !playing; $('play').textContent = playing ? '❚❚ Pause' : '▶ Play'; };
$('reseed').onclick = () => newWorld();
$('speed').oninput = (e) => { speed = +e.target.value; $('speedv').textContent = speed + '×'; };
document.querySelectorAll('.mode-btn').forEach((b) => {
  b.onclick = () => {
    mode = +b.dataset.mode;
    document.querySelectorAll('.mode-btn').forEach((x) => x.classList.toggle('is-on', x === b));
    $('v-watch').textContent = mode === 1 ? 'lineages collapse' : mode === 2 ? 'who is fed' : 'metabolism';
  };
});

fetch('greygoo.wasm')
  .then((r) => r.arrayBuffer())
  .then((bytes) => WebAssembly.instantiate(bytes, {}))
  .then((res) => { wasm = res.instance.exports; newWorld(); requestAnimationFrame(loop); })
  .catch((err) => { $('specimen').innerHTML = '<p style="color:#ff8f8f;font-family:var(--font-mono);padding:24px">Failed to load engine: ' + err + '</p>'; });
