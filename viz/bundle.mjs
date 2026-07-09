// Produce a single self-contained standalone.html with the wasm embedded as
// base64, so it runs offline by double-click (no server, no fetch).
// Usage: node viz/bundle.mjs
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const dir = fileURLToPath(new URL('.', import.meta.url));
const html = await readFile(dir + 'index.html', 'utf8');
const wasm = await readFile(dir + 'greygoo.wasm');
const b64 = wasm.toString('base64');

const patched = html
  .replace(
    "<script>\nconst GENES",
    `<script>\nconst WASM_B64="${b64}";\nconst GENES`
  )
  .replace(
    "fetch('greygoo.wasm')\n  .then(r => r.arrayBuffer())",
    "Promise.resolve(Uint8Array.from(atob(WASM_B64), c => c.charCodeAt(0)).buffer)"
  )
  .replace(
    "<title>Grey Goo — live evolution</title>",
    "<title>Grey Goo — live evolution (standalone)</title>"
  );

await writeFile(dir + 'standalone.html', patched);
console.log(`wrote standalone.html (${(patched.length / 1024).toFixed(0)} KB, wasm ${(b64.length / 1024).toFixed(0)} KB base64)`);
