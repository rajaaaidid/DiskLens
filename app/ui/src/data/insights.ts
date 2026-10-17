import { pathOf } from "./fakeTree";
import type { Category, TreeNode } from "./types";

export interface Hog {
  id: string;
  title: string;
  node: TreeNode;
  size: number;
  /** Plain-English explanation. */
  why: string;
  /** What the user could do about it (HaveSpace never does this automatically). */
  advice: string;
  severity: "info" | "reclaimable" | "system";
}

interface Rule {
  id: string;
  title: string;
  match: (n: TreeNode, path: string) => boolean;
  why: string;
  advice: string;
  severity: Hog["severity"];
  /** Collect every match (e.g. all node_modules) instead of the first. */
  many?: boolean;
}

const lower = (s: string) => s.toLowerCase();

// The Rust core will carry the same table; kept here so the mockup shows real-looking cards.
export const HOG_RULES: Rule[] = [
  {
    id: "hiberfil", title: "Hibernation file", severity: "system",
    match: (n) => lower(n.name) === "hiberfil.sys",
    why: "Windows writes RAM to this file when the PC hibernates or uses Fast Startup. It is roughly 40–75% of your RAM.",
    advice: "If you never hibernate, run `powercfg /h off` in an admin terminal to remove it.",
  },
  {
    id: "pagefile", title: "Page file", severity: "system",
    match: (n) => lower(n.name) === "pagefile.sys",
    why: "Virtual memory that Windows swaps to when RAM runs low. Windows sizes it automatically.",
    advice: "Leave it alone unless you have lots of RAM; it can be capped in System › Advanced › Performance.",
  },
  {
    id: "winsxs", title: "Component store (WinSxS)", severity: "system",
    match: (n) => lower(n.name) === "winsxs",
    why: "Holds every version of Windows components for updates and rollback. Much of it is hard links, so Explorer over-reports it.",
    advice: "Use Disk Cleanup › Windows Update Cleanup. Never delete files from here by hand.",
  },
  {
    id: "windowsold", title: "Previous Windows install", severity: "reclaimable",
    match: (_n, p) => lower(p) === "c:\\windows.old",
    why: "A full copy of your previous Windows version, kept so you can roll back for 10 days after an upgrade.",
    advice: "Safe to remove once you're happy with the upgrade: Settings › System › Storage › Temporary files.",
  },
  {
    id: "recycle", title: "Recycle Bin", severity: "reclaimable",
    match: (n) => lower(n.name) === "$recycle.bin",
    why: "Files you deleted are still taking up space until the Recycle Bin is emptied.",
    advice: "Empty the Recycle Bin once you're sure you don't need anything in it.",
  },
  {
    id: "sysvol", title: "System Volume Information", severity: "system",
    match: (n) => lower(n.name) === "system volume information",
    why: "Stores System Restore points and shadow copies. Its cap is set per drive.",
    advice: "Reduce the cap or delete old restore points in System Protection settings.",
  },
  {
    id: "temp", title: "Temp folders", severity: "reclaimable", many: true,
    match: (n) => n.isDir && lower(n.name) === "temp",
    why: "Leftover files from installers and apps. Most are no longer needed.",
    advice: "Settings › System › Storage › Temporary files clears these safely.",
  },
  {
    id: "browsercache", title: "Browser caches", severity: "reclaimable", many: true,
    match: (n, p) => n.isDir && (lower(n.name) === "cache" || lower(n.name) === "code cache") && /\\(google|microsoft|mozilla|bravesoftware)\\/.test(lower(p)),
    why: "Cached web pages and scripts. Browsers rebuild them automatically.",
    advice: "Clear browsing data from the browser's settings.",
  },
  {
    id: "node_modules", title: "node_modules", severity: "reclaimable", many: true,
    match: (n) => n.isDir && n.name === "node_modules",
    why: "Installed JavaScript dependencies. They can be restored with `npm install` at any time.",
    advice: "Remove them from projects you aren't actively working on.",
  },
  {
    id: "git", title: ".git repositories", severity: "info", many: true,
    match: (n) => n.isDir && n.name === ".git",
    why: "Full version history for a project. Large ones usually contain big binary files.",
    advice: "`git gc` can compact them; don't delete unless the project is pushed elsewhere.",
  },
  {
    id: "vhdx", title: "Docker / WSL virtual disks", severity: "reclaimable", many: true,
    match: (n) => !n.isDir && lower(n.name).endsWith(".vhdx"),
    why: "Virtual disks for WSL distros and Docker. They grow as you use them but never shrink on their own.",
    advice: "Prune Docker images (`docker system prune`) and compact the disk with `wsl --manage <distro> --set-sparse true`.",
  },
  {
    id: "steam", title: "Steam library", severity: "info",
    match: (n) => n.isDir && lower(n.name) === "steamapps",
    why: "Installed games. Each can usually be re-downloaded from Steam.",
    advice: "Uninstall games you're not playing from Steam, or move them to another drive.",
  },
];

export function findHogs(root: TreeNode): Hog[] {
  const out: Hog[] = [];
  const many = new Map<string, TreeNode[]>();
  const walk = (n: TreeNode) => {
    const p = pathOf(n);
    for (const r of HOG_RULES) {
      if (!r.match(n, p)) continue;
      if (r.many) {
        const list = many.get(r.id) ?? [];
        list.push(n);
        many.set(r.id, list);
      } else if (!out.some((h) => h.id === r.id)) {
        out.push({ id: r.id, title: r.title, node: n, size: n.size, why: r.why, advice: r.advice, severity: r.severity });
      }
      // Don't double count nested matches (e.g. Temp inside Temp).
      if (r.many) return;
    }
    for (const c of n.children) if (c.isDir) walk(c);
    for (const c of n.children) if (!c.isDir) {
      const cp = p + "\\" + c.name;
      for (const r of HOG_RULES) {
        if (!r.match(c, cp)) continue;
        if (r.many) many.set(r.id, [...(many.get(r.id) ?? []), c]);
        else if (!out.some((h) => h.id === r.id)) out.push({ id: r.id, title: r.title, node: c, size: c.size, why: r.why, advice: r.advice, severity: r.severity });
      }
    }
  };
  walk(root);
  for (const [id, nodes] of many) {
    const r = HOG_RULES.find((x) => x.id === id)!;
    const biggest = nodes.reduce((a, b) => (b.size > a.size ? b : a));
    const total = nodes.reduce((s, n) => s + n.size, 0);
    out.push({
      id, title: nodes.length > 1 ? `${r.title} ×${nodes.length}` : r.title,
      node: biggest, size: total, why: r.why, advice: r.advice, severity: r.severity,
    });
  }
  return out.sort((a, b) => b.size - a.size);
}

export function largestFiles(root: TreeNode, k = 50): TreeNode[] {
  const all: TreeNode[] = [];
  const walk = (n: TreeNode) => {
    for (const c of n.children) c.isDir ? walk(c) : all.push(c);
  };
  walk(root);
  return all.sort((a, b) => b.size - a.size).slice(0, k);
}

export function categoryTotals(root: TreeNode): { category: Category; size: number; files: number }[] {
  const m = new Map<Category, { size: number; files: number }>();
  const walk = (n: TreeNode) => {
    for (const c of n.children) {
      if (c.isDir) walk(c);
      else {
        const e = m.get(c.category) ?? { size: 0, files: 0 };
        e.size += c.size;
        e.files += 1;
        m.set(c.category, e);
      }
    }
  };
  walk(root);
  return [...m.entries()].map(([category, v]) => ({ category, ...v })).sort((a, b) => b.size - a.size);
}

export function extensionTotals(root: TreeNode, k = 12): { ext: string; size: number; files: number; category: Category }[] {
  const m = new Map<string, { size: number; files: number; category: Category }>();
  const walk = (n: TreeNode) => {
    for (const c of n.children) {
      if (c.isDir) walk(c);
      else {
        const dot = c.name.lastIndexOf(".");
        const ext = dot > 0 ? c.name.slice(dot + 1).toLowerCase() : "(none)";
        const e = m.get(ext) ?? { size: 0, files: 0, category: c.category };
        e.size += c.size;
        e.files += 1;
        m.set(ext, e);
      }
    }
  };
  walk(root);
  return [...m.entries()].map(([ext, v]) => ({ ext, ...v })).sort((a, b) => b.size - a.size).slice(0, k);
}
