// Real-time GPU benchmark of the treemap renderer in headless Edge, driven over CDP.
// Usage: node scripts/bench-ui.mjs [fileCounts...]   (dev server must be running on :1420)
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const edge = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const port = 9333;
const counts = process.argv.slice(2).map(Number);
if (counts.length === 0) counts.push(10000, 60000, 200000);

const proc = spawn(edge, [
  "--headless=new", "--use-angle=d3d11", "--ignore-gpu-blocklist", `--remote-debugging-port=${port}`,
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "havespace-bench-"))}`, "--window-size=1600,980", "about:blank",
], { stdio: "ignore" });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try {
    target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page");
  } catch {}
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let seq = 0;
const pending = new Map();
ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) pending.get(m.id)(m);
};
const send = (method, params = {}) =>
  new Promise((r) => {
    const id = ++seq;
    pending.set(id, r);
    ws.send(JSON.stringify({ id, method, params }));
  });

for (const n of counts) {
  await send("Page.navigate", { url: `http://localhost:1420/?stress=${n}&bench` });
  const res = await send("Runtime.evaluate", {
    awaitPromise: true,
    returnByValue: true,
    expression: `new Promise(r => { const t = setInterval(() => { const e = document.getElementById("bench"); if (e) { clearInterval(t); r(e.textContent); } }, 100); })`,
  });
  console.log(`files=${n}`, res.result?.result?.value);
}
ws.close();
proc.kill();
