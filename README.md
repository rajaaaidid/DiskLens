# DiskLens

A fast disk space analyzer for Windows 11: WizTree-class scanning with a glassmorphism treemap.

> **Status:** milestones (a) scanner + CLI, (b) design mockup and (c) the glass treemap on real scan data in
> Tauri are done. Next up: (d) tree-list ↔ treemap linking, space-hog insights and actions; (e) MFT fast path;
> (f) polish.

![C: drive, light theme](docs/screens/c-drive-light.png)

## Layout

```
crates/disklens-core      scanner engine: arena tree, parallel walk, MFT reader (WIP), treemap layout, categories
crates/disklens-cli       `disklens` command-line tool (scan, bench, fixture)
crates/disklens-fixtures  fake directory tree generator (files, hard links, junctions, symlinks)
app/ui                    React + TypeScript + WebGL2 front end
app/ui/src-tauri          Tauri 2 shell: scan session, binary layout IPC, Mica backdrop
docs/mockup, docs/screens screenshots (fake-data mockup / real app)
scripts                   screenshot, window capture and UI benchmark helpers
```

## Desktop app

```bash
npm --prefix app/ui install
```

Run in development (hot-reloading UI, release-optimised scanner):

```bash
npm --prefix app/ui run tauri dev -- --release
```

Build an installer (`target\release\bundle\nsis\…-setup.exe`). This is not verified yet: packaging is part of
milestone (f), and the Tauri bundler downloads NSIS the first time it runs.

```bash
npm --prefix app/ui run tauri build
```

Command-line options of the app:

- `DiskLens.exe D:\Games` starts scanning that folder or drive immediately.
- `--focus <path>` zooms the treemap to a folder once the scan finishes.
- `--theme light|dark` picks the theme.

How the app works:

- **Streaming:** the scan runs on a background thread. `scan-progress` events reach the UI every 120 ms, and the UI
  pulls a fresh layout at most every 450 ms, so the treemap fills in while the scan runs. Folders that are still
  being read shimmer.
- **Binary layout:** `treemap_layout` returns a binary payload (48 bytes per block, plus names; see
  `src-tauri/src/encode.rs`) rather than JSON. The UI decodes it straight into the WebGL instance buffer.
- **Animation:** zooming plays a 300 ms camera morph. Streamed refreshes and window resizes glide blocks to their new
  positions in 240 ms, and labels move with them.
- **Window:** frameless with a custom title bar. The Mica backdrop comes from `window-vibrancy`, falling back to
  Acrylic on Windows 10. The aurora is drawn at 88% opacity so Mica tints through.

In a plain browser (`npm --prefix app/ui run dev`), the UI uses a mock backend with the fake C: tree. That keeps
design review and screenshots possible without Tauri.

## Prerequisites

- Windows 10/11 x64
- Rust stable (MSVC toolchain): `winget install Rustlang.Rustup`
- Visual Studio 2022 Build Tools with the **Desktop development with C++** workload
- Node.js 20+ (for the UI)

## Scanner CLI

```bash
cargo build --release
```

```bash
target\release\disklens.exe scan C:\ --top 20
```

Options:

| flag | meaning |
|---|---|
| `--method auto\|walk\|findfirst\|mft` | `auto` tries the MFT and falls back to the walk (default) |
| `--threads N` | walk worker threads (default: 2 × logical CPUs, 8–64) |
| `--json` | machine-readable output, including the inaccessible folder list |
| `--quiet` | no live progress line |

Ctrl+C cancels a scan cleanly; partial results are still printed.

Benchmark repeated scans (run 1 is cold-ish, later runs hit the OS cache):

```bash
target\release\disklens.exe bench C:\ --runs 3 --method walk
```

Generate a fake tree to scan:

```bash
target\release\disklens.exe fixture D:\tmp\fake --files 1000000 --depth 12
```

Set `DISKLENS_PROFILE=1` to print the time split between workers and the tree builder.

### How the walk works

- Each folder is opened once and read in 256 KiB batches with
  `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)`. Every entry already carries the logical size,
  size on disk, a 128-bit file ID and the reparse tag, so there are **no per-file stat calls**.
- If a volume doesn't support that call, the scanner downgrades automatically to `FileIdBothDirectoryInfo`, then to
  `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`. FAT drives and some network shares need this. On the
  `FindFirstFileExW` path, size on disk is estimated from the cluster size, and hard links can't be detected.
- **Hard links** are counted once: each file ID is checked against a per-scan set. Every name still shows in the tree.
- **Junctions and symlinks** (any reparse tag with the name-surrogate bit) are recorded but never followed.
  OneDrive placeholders and other non-link reparse points *are* entered.
- Folders that can't be opened are flagged and listed in the scan summary.
- Worker threads enumerate folders. A single builder thread owns the tree, appends each folder's entries as one
  contiguous block of children, and pushes sizes up the ancestor chain right away. This keeps a partially scanned
  tree consistent, which the UI uses to stream results.

### Memory

The tree is stored as columns indexed by `u32` node IDs (`parent`, `first_child`, `child_count`, `size`,
`alloc`, `files`, `dirs`, `mtime`, `ext`, `flags`). All names live in one interned string buffer, and extensions are
interned to `u16`. That works out to about 53 bytes per node plus the name.

Inspect one folder of a scan (useful for checking totals):

```bash
target\release\disklens.exe scan C:\Users\me --show AppData\Local
```

## Benchmarks (this machine)

Ryzen 5 5500 (6C/12T), NVMe SSD, Windows 11, Defender real-time protection on.

| target | files / folders | method | time | peak memory |
|---|---|---|---|---|
| `C:\` (300 GB) | 944k / 240k | walk, cold | 72.8 s | 151 MB |
| `C:\` (300 GB) | 944k / 240k | walk, warm | **20.6 s** | 153 MB |
| `C:\` (318 GB*) | 1.03M / 240k | FindFirstFileEx, warm | 20.8 s | 137 MB |
| `C:\Program Files` | 106k / 12k | walk, warm | 0.5–1.0 s | 23 MB |

| `C:\` in the app, streaming | 962k / 244k | walk, warm | 22–27 s | – |

Wall times on C: vary by roughly ±30% from run to run on this machine, depending on background load. In the same
session the CLI took 25–36 s. The app's tree builder is busy for only about 1.3 s of a scan, so streaming the
layout to the UI costs almost nothing. After the scan, the tree arrays are trimmed to size: 100 MB for 1.2M nodes.

\* FindFirstFileEx counts hard links once per name, so it over-reports C: by 18 GB (83k duplicate names, mostly WinSxS).

What these numbers show:

- **Warm scans are I/O-bound.** The tree builder is busy for only 1.2 s of a 20 s scan. Everything else is spent
  opening and enumerating folders.
- **`C:\Windows` is the bottleneck:** 156k folders at a nearly fixed cost per folder, and wall time stops improving
  beyond 2 threads (1 thread: 23 s; 4–48 threads: about 12 s). Opening folders by file ID instead of by path made no
  difference, so path parsing isn't the cause. The serialization is below the API, most likely file-system filter
  drivers such as Defender on-access scanning.
- **The first cold scan misses the 60 s target.** Warm scans are well under it. The MFT reader (milestone e) avoids
  per-folder opens entirely and is the path meant to deliver the under-10-second goal.

### Treemap renderer

These figures come from headless Edge on an RTX 4060, at 1600×980:

| visible blocks | GPU time per frame | layout (JS, mockup) |
|---|---|---|
| 10,000 | 0.61 ms | 3.9 ms |
| 60,000 | 1.88 ms (~530 fps equivalent) | 24 ms |

The Rust layout handles 60k visible blocks in 2.5 ms. For 900k files it takes 18 ms, because files too small to see
are merged.

## Tests

```bash
cargo test --release --workspace
```

The tests cover:

- **Aggregation:** incremental totals match a full recompute, deleting a folder subtracts its size from every parent,
  and links and hard-link duplicates count zero.
- **Real fixture trees:** hard links, junctions (including one that loops back to its parent), symlinks (skipped
  without Developer Mode), access-denied folders, cancel, live progress, and walk vs FindFirstFileEx parity.
- **Treemap layout:** areas proportional to size, no overlaps, children inside their parent, merging of tiny blocks,
  hit-testing and speed.

## UI mockup

```bash
npm --prefix app/ui install
```

```bash
npm --prefix app/ui run dev
```

Open http://localhost:1420. URL parameters for design review:

- `?theme=light`
- `?zoom=Users/Aidid`
- `?hover=pagefile.sys` (pins the hover state)
- `?stress=60000&fps` (stress test with an FPS overlay)

Screenshots: `scripts\screenshot.ps1 -Query "theme=light" -Out docs\mockup\light.png`.

## Running as administrator (MFT fast path)

Reading the Master File Table requires opening the raw volume (`\\.\C:`), which only an elevated process can do.
The MFT reader lands in milestone (e). After that:

- **CLI:** open *Terminal (Admin)* and run `disklens scan C:\ --method mft`.
- **App:** use the "Restart as admin for fast scan" button, or right-click DiskLens and choose *Run as administrator*.

Without elevation, `--method auto` falls back to the walk and says why.
