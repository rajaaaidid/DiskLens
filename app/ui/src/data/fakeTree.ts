import { categoryForName } from "./categories";
import type { Category, Drive, TreeNode } from "./types";

// Deterministic fake C:\ drive for the design mockup. Shapes mimic a real Windows 11 machine.

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const KB = 1024;

function mulberry32(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const rand = mulberry32(0xd15c1e75);
const pick = <T,>(xs: T[]) => xs[Math.floor(rand() * xs.length)];
/** Log-normal-ish size around `median`. */
const sizeAround = (median: number, spread = 1.2) => Math.max(1, Math.round(median * Math.exp((rand() * 2 - 1) * spread * 1.5)));

const NOW = Date.UTC(2026, 9, 17);
const randDate = () => NOW - Math.floor(rand() * 900) * 86_400_000;

let nextId = 1;

type Spec = { d: string; c: Spec[] } | { f: string; s: number; cat?: Category } | { gen: GenSpec };

interface GenSpec {
  prefix: string;
  exts: string[];
  count: number;
  median: number;
  spread?: number;
  /** Spread the files across this many generated subfolders, recursively. */
  fanout?: number;
  depth?: number;
}

const d = (name: string, ...c: Spec[]): Spec => ({ d: name, c });
const f = (name: string, size: number, cat?: Category): Spec => ({ f: name, s: size, cat });
const gen = (prefix: string, exts: string[], count: number, median: number, opts: Partial<GenSpec> = {}): Spec => ({
  gen: { prefix, exts, count, median, ...opts },
});

const WORDS = [
  "assets", "cache", "data", "bin", "lib", "res", "media", "shared", "core", "runtime",
  "modules", "packages", "content", "textures", "shaders", "locale", "plugins", "build",
  "x64", "common", "redist", "logs", "backup", "maps", "audio", "video", "ui", "fonts",
];

function makeFile(name: string, size: number, parent: TreeNode, cat?: Category): TreeNode {
  return {
    id: nextId++,
    name,
    size,
    onDisk: Math.ceil(size / 4096) * 4096,
    files: 1,
    isDir: false,
    category: cat ?? categoryForName(name),
    dominant: cat ?? categoryForName(name),
    mtime: randDate(),
    parent,
    children: [],
  };
}

function makeDir(name: string, parent: TreeNode | null): TreeNode {
  return {
    id: nextId++,
    name,
    size: 0,
    onDisk: 0,
    files: 0,
    isDir: true,
    category: "folder",
    dominant: "other",
    mtime: randDate(),
    parent,
    children: [],
  };
}

function expandGen(g: GenSpec, parent: TreeNode, depth = 0) {
  const fanout = g.fanout ?? 0;
  const maxDepth = g.depth ?? 2;
  if (fanout > 0 && depth < maxDepth && g.count > 40) {
    // Split files: some stay here, the rest go into subfolders.
    const here = Math.floor(g.count * 0.15);
    for (let i = 0; i < here; i++) {
      parent.children.push(makeFile(`${g.prefix}_${i}.${pick(g.exts)}`, sizeAround(g.median, g.spread), parent));
    }
    const rest = g.count - here;
    const used = new Set<string>();
    for (let k = 0; k < fanout; k++) {
      let name = pick(WORDS);
      while (used.has(name)) name = `${pick(WORDS)}${k}`;
      used.add(name);
      const sub = makeDir(name, parent);
      parent.children.push(sub);
      // Uneven split so some subfolders dominate.
      const share = Math.max(1, Math.round((rest / fanout) * (0.3 + rand() * 1.4)));
      expandGen({ ...g, count: share, prefix: `${name}` }, sub, depth + 1);
    }
    return;
  }
  for (let i = 0; i < g.count; i++) {
    parent.children.push(makeFile(`${g.prefix}_${i}.${pick(g.exts)}`, sizeAround(g.median, g.spread), parent));
  }
}

function build(spec: Spec, parent: TreeNode | null): TreeNode | null {
  if ("d" in spec) {
    const node = makeDir(spec.d, parent);
    for (const c of spec.c) {
      if ("gen" in c) expandGen(c.gen, node);
      else {
        const child = build(c, node);
        if (child) node.children.push(child);
      }
    }
    return node;
  }
  if ("f" in spec) return makeFile(spec.f, spec.s, parent!, spec.cat);
  return null;
}

function aggregate(n: TreeNode): void {
  if (!n.isDir) return;
  let size = 0;
  let onDisk = 0;
  let files = 0;
  let mtime = 0;
  const byCat = new Map<Category, number>();
  for (const c of n.children) {
    aggregate(c);
    size += c.size;
    onDisk += c.onDisk;
    files += c.files;
    mtime = Math.max(mtime, c.mtime);
    byCat.set(c.dominant, (byCat.get(c.dominant) ?? 0) + c.size);
  }
  let best = 0;
  for (const [cat, s] of byCat) if (s > best) ((best = s), (n.dominant = cat));
  n.size = size;
  n.onDisk = onDisk;
  n.files = files;
  n.mtime = mtime || n.mtime;
}

const game = (name: string, sizeGb: number) =>
  d(
    name,
    gen("pak", ["pak", "vpk", "bin"], 30, (sizeGb * GB * 0.7) / 30, { spread: 0.6, fanout: 3, depth: 1 }),
    gen("tex", ["dds", "bin", "pak"], 200, (sizeGb * GB * 0.25) / 200, { fanout: 4, depth: 2 }),
    gen("lib", ["dll", "exe"], 40, (sizeGb * GB * 0.05) / 40),
  );

const project = (name: string) =>
  d(
    name,
    d(".git", gen("pack", ["pack", "idx"], 12, 18 * MB, { spread: 1.5 }), gen("obj", ["obj"], 300, 6 * KB)),
    d("node_modules", gen("mod", ["js", "map", "json", "ts"], 3200, 9 * KB, { fanout: 8, depth: 3 })),
    d("src", gen("src", ["ts", "tsx", "rs", "json"], 160, 7 * KB, { fanout: 4 })),
    d("target", gen("build", ["rlib", "pdb", "exe", "obj"], 400, 900 * KB, { fanout: 3 })),
  );

const SPEC: Spec = d(
  "C:\\",
  f("pagefile.sys", 16 * GB, "system"),
  f("hiberfil.sys", Math.round(12.7 * GB), "system"),
  f("swapfile.sys", 256 * MB, "system"),
  f("DumpStack.log.tmp", 8 * KB, "system"),
  d("$Recycle.Bin", d("S-1-5-21-3141", gen("$R", ["zip", "mp4", "exe", "docx", "png"], 60, 40 * MB, { spread: 1.6 }))),
  d("System Volume Information", gen("{3808876b}", ["dat"], 8, 1.1 * GB, { spread: 0.4 })),
  d(
    "Windows",
    d("WinSxS", gen("amd64_microsoft-windows", ["dll", "mui", "manifest", "cat", "sys"], 9000, 1.1 * MB, { fanout: 12, depth: 2 })),
    d("System32", gen("sys", ["dll", "exe", "mui", "sys", "dat"], 4200, 1.2 * MB, { fanout: 10, depth: 2 })),
    d("SysWOW64", gen("wow", ["dll", "exe"], 1800, 900 * KB, { fanout: 6 })),
    d("Installer", gen("inst", ["msi", "msp", "cab"], 220, 22 * MB, { spread: 1.4 })),
    d("SoftwareDistribution", d("Download", gen("kb", ["cab", "esd", "psf"], 60, 70 * MB, { spread: 1.3 }))),
    d("Temp", gen("tmp", ["tmp", "log", "etl"], 400, 1.2 * MB, { spread: 1.6 })),
    d("Fonts", gen("font", ["ttf", "otf"], 300, 400 * KB)),
    d("Logs", gen("log", ["log", "etl", "evtx"], 500, 900 * KB)),
  ),
  d(
    "Windows.old",
    d("Windows", gen("old", ["dll", "exe", "mui", "sys", "wim"], 3000, 4.5 * MB, { fanout: 8, depth: 2 })),
    d("Program Files", gen("oldapp", ["dll", "exe", "pak"], 600, 6 * MB, { fanout: 6 })),
  ),
  d(
    "Program Files",
    d("Adobe", gen("adobe", ["dll", "exe", "pak", "zip"], 1100, 3.2 * MB, { fanout: 5, depth: 2 })),
    d("Microsoft Office", gen("office", ["dll", "exe", "msi"], 900, 3.4 * MB, { fanout: 5 })),
    d("NVIDIA Corporation", gen("nv", ["dll", "exe", "bin"], 300, 6 * MB, { fanout: 4 })),
    d("Docker", gen("docker", ["exe", "dll", "iso"], 80, 25 * MB, { spread: 1.6 })),
    d("Blender Foundation", gen("blender", ["dll", "py", "exe"], 1600, 600 * KB, { fanout: 6 })),
    d("JetBrains", gen("jb", ["jar", "dll", "exe"], 1500, 1.4 * MB, { fanout: 6 })),
    d("WindowsApps", gen("app", ["msix", "dll", "exe", "pak"], 1400, 3 * MB, { fanout: 10 })),
  ),
  d(
    "Program Files (x86)",
    d(
      "Steam",
      d(
        "steamapps",
        d("common", game("Starfall Odyssey", 46), game("Iron Tide", 23), game("Hollow Lanterns", 6), game("Neon Drift Racing", 12), game("Pixel Farm", 1.2)),
        d("shadercache", gen("shader", ["bin"], 400, 1.6 * MB, { fanout: 5 })),
        d("downloading", gen("chunk", ["pak"], 30, 120 * MB)),
      ),
      gen("steam", ["dll", "exe"], 140, 2 * MB),
    ),
    d("Microsoft", gen("ms", ["dll", "exe"], 900, 1.6 * MB, { fanout: 6 })),
    d("Common Files", gen("common", ["dll"], 700, 900 * KB, { fanout: 5 })),
  ),
  d(
    "ProgramData",
    d("Microsoft", gen("pd", ["dat", "log", "etl", "db"], 1200, 900 * KB, { fanout: 6 })),
    d("Package Cache", gen("pkg", ["msi", "cab", "exe"], 260, 14 * MB, { spread: 1.3, fanout: 4 })),
    d("NVIDIA", gen("nvcache", ["bin", "dat"], 300, 2 * MB)),
  ),
  d(
    "Users",
    d("Public", gen("pub", ["lnk", "jpg", "ini"], 60, 300 * KB)),
    d(
      "Aidid",
      d("Videos", d("Captures", gen("clip", ["mp4"], 70, 260 * MB, { spread: 0.9 })), d("Projects", gen("render", ["mp4", "mov", "mkv"], 14, 1.1 * GB, { spread: 0.7 })), gen("movie", ["mkv", "mp4"], 6, 2.1 * GB, { spread: 0.4 })),
      d("Pictures", d("Camera Roll", gen("IMG", ["jpg", "heic", "png"], 2600, 3.2 * MB, { fanout: 6 })), d("Screenshots", gen("Screenshot", ["png"], 1400, 1.1 * MB)), d("RAW", gen("DSC", ["cr2", "nef"], 700, 26 * MB, { fanout: 4 }))),
      d("Music", gen("track", ["flac", "mp3"], 1500, 9 * MB, { fanout: 8 })),
      d("Downloads", gen("download", ["zip", "exe", "msi", "iso", "pdf", "mp4", "7z", "png"], 260, 14 * MB, { spread: 1.7 })),
      d("Documents", gen("doc", ["docx", "pdf", "xlsx", "pptx", "txt"], 900, 900 * KB, { fanout: 6 }), d("Projects", project("disklens"), project("abyss-salvage"), project("web-portfolio"))),
      d("OneDrive", gen("sync", ["docx", "pdf", "jpg", "xlsx"], 800, 1.6 * MB, { fanout: 5 })),
      d(
        "AppData",
        d(
          "Local",
          d("Docker", d("wsl", d("disk", f("docker_data.vhdx", Math.round(31.4 * GB), "system")), d("main", f("ext4.vhdx", Math.round(2.1 * GB), "system")))),
          d("Packages", d("CanonicalGroupLimited.Ubuntu", d("LocalState", f("ext4.vhdx", Math.round(14.6 * GB), "system")))),
          d("Google", d("Chrome", d("User Data", d("Default", d("Cache", gen("f_", ["dat"], 2400, 400 * KB, { fanout: 4 })), d("Code Cache", gen("js", ["dat"], 900, 300 * KB)))))),
          d("Microsoft", d("Edge", d("User Data", d("Default", d("Cache", gen("f_", ["dat"], 1400, 350 * KB))))), d("Teams", gen("teams", ["dat", "log"], 500, 1 * MB))),
          d("Temp", gen("tmp", ["tmp", "log", "zip", "msi"], 1800, 900 * KB, { spread: 1.8, fanout: 6 })),
          d("npm-cache", gen("content", ["tgz", "json"], 4000, 120 * KB, { fanout: 8 })),
          d("NVIDIA", d("DXCache", gen("dx", ["bin"], 600, 2 * MB))),
        ),
        d("Roaming", d("Code", gen("vscode", ["json", "log", "db", "vsix"], 800, 600 * KB, { fanout: 5 })), d("Discord", gen("discord", ["dat", "log"], 300, 900 * KB))),
      ),
    ),
  ),
);

export const FAKE_ROOT: TreeNode = (() => {
  const root = build(SPEC, null)!;
  aggregate(root);
  return root;
})();

export const FAKE_DRIVES: Drive[] = [
  { letter: "C:", label: "Windows", fs: "NTFS", total: FAKE_ROOT.size + 92 * GB, free: 92 * GB },
  { letter: "D:", label: "Data", fs: "NTFS", total: 161_325_510_656, free: 66_157_002_752 },
  { letter: "E:", label: "Games", fs: "NTFS", total: 484_784_992_256, free: 185_028_866_048 },
  { letter: "G:", label: "Archive", fs: "NTFS", total: 838_860_795_904, free: 197_006_057_472 },
];

export function countNodes(n: TreeNode): number {
  let c = 1;
  for (const ch of n.children) c += countNodes(ch);
  return c;
}

export function pathOf(n: TreeNode): string {
  const parts: string[] = [];
  for (let cur: TreeNode | null = n; cur; cur = cur.parent) parts.push(cur.name);
  parts.reverse();
  return parts[0] + parts.slice(1).join("\\");
}


/** Synthetic two-level tree with `files` leaves, for renderer stress tests (`?stress=60000`). */
export function makeStressTree(files: number): TreeNode {
  const root = makeDir("C:\\", null);
  const per = Math.max(1, Math.round(files / 64));
  for (let i = 0; i < 8; i++) {
    const d = makeDir(`group_${i}`, root);
    root.children.push(d);
    for (let j = 0; j < 8; j++) {
      const s = makeDir(`dir_${i}_${j}`, d);
      d.children.push(s);
      for (let k = 0; k < per; k++) {
        const ext = pick(["mp4", "png", "zip", "exe", "pdf", "dll", "rs", "dat"]);
        s.children.push(makeFile(`f_${k}.${ext}`, 1_000_000 + Math.round(rand() * 400_000), s));
      }
    }
  }
  aggregate(root);
  return root;
}