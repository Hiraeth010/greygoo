// Grey Goo — site driver. Loads the real engine (WASM), runs the live culture,
// mirrors the population genome as telemetry, and reveals sections on scroll.
document.documentElement.classList.add('js');

const GENES = ['metab', 'repro', 'move', 'aggr', 'mut', 'aff', 'split', 'dorm'];
const HOT = new Set(['metab', 'aff', 'aggr']); // the genes under strongest selection
const W = 190, H = 190, INIT_AGENTS = 7000;

const $ = (id) => document.getElementById(id);
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

// ---- build gene telemetry rows ----
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
  (entries) => entries.forEach((e) => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }),
  { threshold: 0.16, rootMargin: '0px 0px -8% 0px' }
);
document.querySelectorAll('.phase, .reveal').forEach((el) => {
  if (reduceMotion) el.classList.add('in'); else io.observe(el);
});

// ---- the engine ----
const cv = $('grid');
const ctx = cv.getContext('2d');
ctx.imageSmoothingEnabled = false;

let wasm, app, imgData, off, offCtx;
let playing = true, speed = 4, frame = 0;

function u32(f) { return f >>> 0; }

function newWorld() {
  if (app) wasm.free_app(app);
  const seed = (Math.floor(performance.now() * 1000) ^ 0x9e3779b9) >>> 0;
  app = wasm.init(u32(seed), 0, W, H, INIT_AGENTS);
  off = document.createElement('canvas'); off.width = W; off.height = H;
  offCtx = off.getContext('2d');
  imgData = offCtx.createImageData(W, H);
}

function draw() {
  const ptr = wasm.render(app);
  imgData.data.set(new Uint8Array(wasm.memory.buffer, ptr, W * H * 4));
  offCtx.putImageData(imgData, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  ctx.drawImage(off, 0, 0, cv.width, cv.height);

  // telemetry — throttle the heavier reads
  if (frame % 3 === 0) {
    $('r-epoch').textContent = wasm.epoch(app).toLocaleString();
    $('r-pop').textContent = wasm.population(app).toLocaleString();
    GENES.forEach((name, i) => {
      const m = wasm.gene_mean(app, i);
      rows[name].fill.style.width = (m / 255 * 100).toFixed(1) + '%';
      rows[name].val.textContent = m.toFixed(0);
    });
  }
  if (frame % 12 === 0) $('r-strain').textContent = wasm.strains(app).toLocaleString();
}

function loop() {
  if (playing) wasm.step(app, speed);
  draw();
  frame++;
  requestAnimationFrame(loop);
}

$('play').onclick = () => {
  playing = !playing;
  $('play').textContent = playing ? '❚❚ Pause' : '▶ Play';
};
$('reseed').onclick = () => newWorld();
$('speed').oninput = (e) => { speed = +e.target.value; $('speedv').textContent = speed + ' ticks/f'; };

fetch('greygoo.wasm')
  .then((r) => r.arrayBuffer())
  .then((bytes) => WebAssembly.instantiate(bytes, {}))
  .then((res) => {
    wasm = res.instance.exports;
    newWorld();
    requestAnimationFrame(loop);
  })
  .catch((err) => {
    $('specimen').innerHTML =
      '<p style="color:#ff8f8f;font-family:var(--font-mono);padding:24px">Failed to load engine: ' + err + '</p>';
  });
