// Drive the browser build (mock backend) through the milestone (d) interactions with real input events
// in headless Edge, saving a screenshot per step. Usage: node scripts/ui-flow.mjs [outDir]
// (dev server must be running on :1420)
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

//
// `--attach=PORT` drives the real desktop app instead: start it with
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=PORT
// (it must already be scanning a throwaway folder: step 6 really uses the Recycle Bin).
const attach = process.argv.find((a) => a.startsWith("--attach="))?.split("=")[1];
const outDir = process.argv.slice(2).find((a) => !a.startsWith("--")) ?? join(tmpdir(), "disklens-flow");
mkdirSync(outDir, { recursive: true });
const edge = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const port = attach ? Number(attach) : 9334;
const proc = attach
  ? null
  : spawn(edge, [
      "--headless=new", "--use-angle=d3d11", "--ignore-gpu-blocklist", `--remote-debugging-port=${port}`,
      `--user-data-dir=${mkdtempSync(join(tmpdir(), "disklens-flow-"))}`, "--window-size=1600,980", "about:blank",
    ], { stdio: "ignore" });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try {
    target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page" && (!attach || t.url.includes("1420")));
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
const key = async (k) => {
  await send("Input.dispatchKeyEvent", { type: "keyDown", key: k, code: k, windowsVirtualKeyCode: k === "Delete" ? 46 : k === "Escape" ? 27 : 0 });
  await send("Input.dispatchKeyEvent", { type: "keyUp", key: k, code: k });
};
const rectOf = (sel) => evaluate(`(() => { const r = document.querySelector(${JSON.stringify(sel)})?.getBoundingClientRect(); return r && { x: r.left, y: r.top, w: r.width, h: r.height }; })()`);
const shot = async (name) => {
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(outDir, `${name}.png`), Buffer.from(r.result.data, "base64"));
  console.log("shot", name);
};
const text = (sel) => evaluate(`document.querySelector(${JSON.stringify(sel)})?.innerText ?? null`);

if (!attach) {
  await send("Emulation.setDeviceMetricsOverride", { width: 1600, height: 980, deviceScaleFactor: 1, mobile: false });
  await send("Page.navigate", { url: "http://localhost:1420/" });
}
// Wait for the scan to finish (the method badge appears with the summary).
for (let i = 0; i < 600 && !(await evaluate("!!document.querySelector('.scan-card .badge')")); i++) await sleep(100);
await sleep(600);
const tm = await rectOf(".treemap-host");
const log = {};
log.rootBefore = await text(".crumb-size");

// 1. Hover a block.
await move(tm.x + tm.w * 0.62, tm.y + tm.h * 0.75);
await sleep(400);
log.tooltip = await text(".tooltip .tt-title");
await shot("1-hover");

// 2. Click it: zooms into the top-level folder and selects the clicked item.
await click(tm.x + tm.w * 0.62, tm.y + tm.h * 0.75);
await sleep(900);
log.crumbsAfterClick = await evaluate("[...document.querySelectorAll('.crumb')].map(c => c.innerText.trim())");
log.selectedRow = await text(".tl-row.selected .name-text");
await shot("2-click-zoom-select");

// 3. Hover a tree-list row: the matching block lights up.
const row = await rectOf(".tl-row:nth-child(2)");
await move(row.x + 120, row.y + row.h / 2);
await sleep(400);
await shot("3-row-hover-links-block");

// 4. Right-click a block: context menu.
await move(tm.x + tm.w * 0.3, tm.y + tm.h * 0.4);
await sleep(150);
await click(tm.x + tm.w * 0.3, tm.y + tm.h * 0.4, "right");
await sleep(500);
log.menuTitle = await text(".ctx-title");
log.menuItems = await evaluate("[...document.querySelectorAll('.ctx-menu button')].map(b => (b.disabled ? '(disabled) ' : '') + b.innerText.trim())");
await shot("4-context-menu");

// 5. Choose "Send to Recycle Bin…": confirmation dialog, Cancel focused.
const del = await rectOf(".ctx-menu button.danger");
if (del) {
  await click(del.x + 40, del.y + del.h / 2);
  await sleep(400);
}
log.dialog = await text(".modal h2");
log.dialogFocus = await evaluate("document.activeElement?.innerText ?? null");
await shot("5-confirm-dialog");

// 6. Confirm: item disappears, totals update, toast shows.
const ok = await rectOf(".btn-danger");
if (ok) {
  await click(ok.x + ok.w / 2, ok.y + ok.h / 2);
  await sleep(900);
}
log.toast = await evaluate("[...document.querySelectorAll('.toast')].map(t => t.innerText)");
log.rootAfter = await text(".crumb-size");
await shot("6-after-recycle");

// 7. Escape back to the root, then a protected item: Windows → menu shows the reason.
await key("Escape");
await sleep(700);
log.crumbsAfterEsc = await evaluate("[...document.querySelectorAll('.crumb')].map(c => c.innerText.trim())");

// 8. Insights tabs.
await shot("7-hogs");
const types = await evaluate("(() => { const b = [...document.querySelectorAll('.panel-head .segmented button')].find(b => b.innerText === 'Types'); const r = b.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()");
await click(types.x, types.y);
await sleep(300);
await shot("8-types");

writeFileSync(join(outDir, "log.json"), JSON.stringify(log, null, 2));
console.log(JSON.stringify(log, null, 2));
ws.close();
proc?.kill();
