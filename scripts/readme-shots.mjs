// Recapture the README screenshots (docs/screens) from the browser build's fake C: drive, so no real
// disk contents end up in the repo. Usage: node scripts/readme-shots.mjs [outDir]
// (dev server must be running on :1420; `?shots` hides the demo-data notice)
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const outDir = process.argv[2] ?? "docs/screens";
mkdirSync(outDir, { recursive: true });
const edge = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const port = 9335;
const proc = spawn(edge, [
  "--headless=new", "--use-angle=d3d11", "--ignore-gpu-blocklist", `--remote-debugging-port=${port}`,
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "disklens-shots-"))}`, "--window-size=1600,980", "about:blank",
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
const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result?.result?.value;
const mouse = (type, x, y, extra = {}) => send("Input.dispatchMouseEvent", { type, x, y, button: "none", ...extra });
const move = (x, y) => mouse("mouseMoved", x, y);
const click = async (x, y, button = "left") => {
  await move(x, y);
  await mouse("mousePressed", x, y, { button, clickCount: 1 });
  await mouse("mouseReleased", x, y, { button, clickCount: 1 });
};
const rectOf = (sel) => evaluate(`(() => { const r = document.querySelector(${JSON.stringify(sel)})?.getBoundingClientRect(); return r && { x: r.left, y: r.top, w: r.width, h: r.height }; })()`);
const shot = async (name) => {
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, `${name}.png`), Buffer.from(r.result.data, "base64"));
  console.log("shot", name);
};
const open = async (query, waitForScan = true) => {
  await send("Page.navigate", { url: `http://localhost:1420/?shots&${query}` });
  await sleep(500);
  if (waitForScan) {
    for (let i = 0; i < 600 && !(await evaluate("!!document.querySelector('.scan-card .badge')")); i++) await sleep(100);
    await sleep(800);
  }
};

await send("Emulation.setDeviceMetricsOverride", { width: 1600, height: 980, deviceScaleFactor: 1, mobile: false });

await open("picker", false);
await sleep(1200);
await shot("drive-picker");

await open("", false);
await sleep(700);
await shot("streaming");

await open("theme=light");
await shot("c-drive-light");

await open("zoom=Users/Alex/AppData/Local");
await shot("appdata-local-dark");

await open("");
await shot("space-hogs");
const types = await evaluate("(() => { const b = [...document.querySelectorAll('.panel-head .segmented button')].find(b => b.innerText === 'Types'); const r = b.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()");
await click(types.x, types.y);
await sleep(300);
await shot("types");

// Click a block (zooms into its top-level folder and selects it), then hover a tree-list row.
await open("");
const tm = await rectOf(".treemap-host");
await click(tm.x + tm.w * 0.62, tm.y + tm.h * 0.75);
await sleep(900);
const row = await rectOf(".tl-row:nth-child(2)");
await move(row.x + 120, row.y + row.h / 2);
await sleep(400);
await shot("select-linked");

await move(tm.x + tm.w * 0.3, tm.y + tm.h * 0.4);
await sleep(150);
await click(tm.x + tm.w * 0.3, tm.y + tm.h * 0.4, "right");
await sleep(500);
await shot("context-menu");

const del = await rectOf(".ctx-menu button.danger");
if (del) {
  await click(del.x + 40, del.y + del.h / 2);
  await sleep(500);
}
await shot("confirm-recycle");

// Design mockups (docs/mockup): root with a pinned hover, and zoomed into the Steam library.
const mockDir = join(outDir, "..", "mockup");
mkdirSync(mockDir, { recursive: true });
const steam = "zoom=Program Files (x86)/Steam/steamapps/common";
for (const [name, query] of [
  ["dark", "hover=pagefile.sys"],
  ["light", "theme=light&hover=pagefile.sys"],
  ["dark-zoomed", steam],
  ["light-zoomed", `theme=light&${steam}`],
]) {
  await open(query);
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(mockDir, `${name}.png`), Buffer.from(r.result.data, "base64"));
  console.log("mockup", name);
}

ws.close();
proc.kill();
