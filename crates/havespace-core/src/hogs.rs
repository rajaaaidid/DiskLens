//! Known space hogs, explained in plain English.
//!
//! One pass over the tree. Rules that can match many folders (Temp, node_modules, caches...) count only
//! the top-most match, so `Temp\Temp` is not counted twice.

use serde::Serialize;

use crate::tree::{flags, NodeId, Tree, NONE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Usually safe to clean up with the advice given.
    Reclaimable,
    /// Managed by Windows; shrink it through Windows settings, never by hand.
    System,
    /// Worth knowing about; your call.
    Info,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HogItem {
    pub node: NodeId,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hog {
    pub id: &'static str,
    pub title: &'static str,
    pub why: &'static str,
    pub advice: &'static str,
    pub severity: Severity,
    /// Whether offering "Send to Recycle Bin" for the items makes sense.
    pub recyclable: bool,
    pub size: u64,
    pub count: u32,
    /// Largest matches first (at most 8).
    pub items: Vec<HogItem>,
}

struct Ctx<'a> {
    t: &'a Tree,
    id: NodeId,
    name: String,
    parent_name: String,
    is_dir: bool,
    parent_is_root: bool,
}

impl Ctx<'_> {
    fn path_lower(&self) -> String {
        self.t.path(self.id).to_ascii_lowercase()
    }
}

struct Rule {
    id: &'static str,
    title: &'static str,
    why: &'static str,
    advice: &'static str,
    severity: Severity,
    recyclable: bool,
    matches: fn(&Ctx) -> bool,
}

const BROWSER_MARKERS: [&str; 6] = ["\\google\\chrome\\", "\\microsoft\\edge\\", "\\bravesoftware\\", "\\mozilla\\firefox\\", "\\opera software\\", "\\vivaldi\\"];

const RULES: &[Rule] = &[
    Rule {
        id: "hiberfil",
        title: "Hibernation file",
        why: "Windows saves the contents of RAM here when the PC hibernates or uses Fast Startup. It is typically 40–75% of your RAM.",
        advice: "If you never hibernate, run `powercfg /h off` in an administrator terminal and Windows removes it.",
        severity: Severity::System,
        recyclable: false,
        matches: |c| !c.is_dir && c.name == "hiberfil.sys",
    },
    Rule {
        id: "pagefile",
        title: "Page and swap files",
        why: "Virtual memory Windows uses when RAM runs low. Windows sizes these automatically.",
        advice: "Leave them alone unless you have plenty of RAM; the size can be capped in System › Advanced system settings › Performance › Virtual memory.",
        severity: Severity::System,
        recyclable: false,
        matches: |c| !c.is_dir && (c.name == "pagefile.sys" || c.name == "swapfile.sys"),
    },
    Rule {
        id: "winsxs",
        title: "Windows component store (WinSxS)",
        why: "Every version of every Windows component, kept for updates and rollback. Much of it is hard links into System32, so it is smaller than Explorer claims.",
        advice: "Use Disk Cleanup › Clean up system files › Windows Update Cleanup, or `Dism /Online /Cleanup-Image /StartComponentCleanup`. Never delete files from here by hand.",
        severity: Severity::System,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "winsxs" && c.parent_name == "windows",
    },
    Rule {
        id: "windowsold",
        title: "Previous Windows installation",
        why: "A full copy of your previous Windows version, kept so you can roll back for 10 days after a feature update.",
        advice: "Once you're happy with the update, remove it in Settings › System › Storage › Temporary files › Previous Windows installation(s).",
        severity: Severity::Reclaimable,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "windows.old" && c.parent_is_root,
    },
    Rule {
        id: "recyclebin",
        title: "Recycle Bin",
        why: "Files you deleted are still on disk until the Recycle Bin is emptied.",
        advice: "Open the Recycle Bin, check nothing in it is still needed, then empty it.",
        severity: Severity::Reclaimable,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "$recycle.bin",
    },
    Rule {
        id: "sysvol",
        title: "System Restore and shadow copies",
        why: "System Volume Information holds restore points and volume shadow copies. Its maximum size is set per drive.",
        advice: "Lower the limit or delete old restore points in System Properties › System Protection › Configure.",
        severity: Severity::System,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "system volume information",
    },
    Rule {
        id: "temp",
        title: "Temporary files",
        why: "Leftovers from installers, updates and apps. Most of it is no longer needed once the program that made it has finished.",
        advice: "Settings › System › Storage › Temporary files clears these safely. Files in use are skipped.",
        severity: Severity::Reclaimable,
        recyclable: true,
        matches: |c| c.is_dir && (c.name == "temp" || c.name == "tmp"),
    },
    Rule {
        id: "browsercache",
        title: "Browser caches",
        why: "Copies of web pages, images and scripts that browsers keep to load sites faster. They are rebuilt automatically.",
        advice: "Clear cached images and files from the browser's settings (close the browser first if you remove them by hand).",
        severity: Severity::Reclaimable,
        recyclable: true,
        matches: |c| {
            c.is_dir
                && matches!(c.name.as_str(), "cache" | "code cache" | "gpucache" | "cache_data" | "cache2" | "shadercache" | "grshadercache")
                && {
                    let p = c.path_lower();
                    BROWSER_MARKERS.iter().any(|m| p.contains(m))
                }
        },
    },
    Rule {
        id: "wupdate",
        title: "Windows Update downloads",
        why: "Update packages Windows has already downloaded. After updates install, they are no longer needed.",
        advice: "Disk Cleanup › Clean up system files › Windows Update Cleanup removes them safely.",
        severity: Severity::System,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "download" && c.parent_name == "softwaredistribution",
    },
    Rule {
        id: "node_modules",
        title: "node_modules folders",
        why: "Installed JavaScript packages. Any project can restore them with `npm install`.",
        advice: "Remove them from projects you aren't working on; reinstall when you need them.",
        severity: Severity::Reclaimable,
        recyclable: true,
        matches: |c| c.is_dir && c.name == "node_modules",
    },
    Rule {
        id: "git",
        title: "Git repositories",
        why: "The full history of a project. Large ones usually contain big binary files that were committed at some point.",
        advice: "`git gc` compacts a repository. Only delete one if the project is safely pushed elsewhere.",
        severity: Severity::Info,
        recyclable: false,
        matches: |c| c.is_dir && c.name == ".git",
    },
    Rule {
        id: "vhdx",
        title: "Docker / WSL virtual disks",
        why: "Virtual disks for WSL distributions and Docker. They grow as you use them but never shrink on their own.",
        advice: "Run `docker system prune`, then compact the disk (`wsl --manage <distro> --set-sparse true`, or Optimize-VHD).",
        severity: Severity::Reclaimable,
        recyclable: false,
        matches: |c| !c.is_dir && (c.name.ends_with(".vhdx") || c.name.ends_with(".vhd")),
    },
    Rule {
        id: "steam",
        title: "Steam libraries",
        why: "Installed games. Every one can be downloaded again from your Steam library.",
        advice: "Uninstall games you're not playing from Steam, or move them to another drive (Steam › Settings › Storage).",
        severity: Severity::Info,
        recyclable: false,
        matches: |c| c.is_dir && c.name == "steamapps",
    },
];

/// Find all known space hogs in the live part of the tree, largest first.
pub fn find_hogs(t: &Tree) -> Vec<Hog> {
    assert!(RULES.len() <= 32);
    let live = t.live_mask();
    let n = t.len();
    // Rules matched by a node or any of its ancestors (for "top-most only").
    let mut seen = vec![0u32; n];
    let mut acc: Vec<(u64, u32, Vec<(u64, NodeId)>)> = RULES.iter().map(|_| (0, 0, Vec::new())).collect();
    let root_is_drive = t.name(0).len() == 3 && t.name(0).ends_with(":\\");

    for i in 1..n {
        let id = i as NodeId;
        let p = t.parent[i];
        let inherited = if p == NONE { 0 } else { seen[p as usize] };
        seen[i] = inherited;
        if !live[i] || t.has_flag(id, flags::LINK | flags::HARDLINK_DUP) {
            continue;
        }
        let name = t.name(id);
        // Cheap pre-filter: every rule keys off a handful of names/extensions.
        let lower = name.to_ascii_lowercase();
        let ctx = Ctx {
            t,
            id,
            parent_name: if p == NONE { String::new() } else { t.name(p).to_ascii_lowercase() },
            name: lower,
            is_dir: t.is_dir(id),
            parent_is_root: p == 0 && root_is_drive,
        };
        for (r, rule) in RULES.iter().enumerate() {
            let bit = 1u32 << r;
            if inherited & bit != 0 || !(rule.matches)(&ctx) {
                continue;
            }
            seen[i] |= bit;
            let size = t.size[i];
            if size == 0 {
                continue;
            }
            let a = &mut acc[r];
            a.0 += size;
            a.1 += 1;
            a.2.push((size, id));
        }
    }

    let mut out: Vec<Hog> = RULES
        .iter()
        .zip(acc)
        .filter(|(_, a)| a.1 > 0)
        .map(|(rule, (size, count, mut items))| {
            items.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            items.truncate(8);
            Hog {
                id: rule.id,
                title: rule.title,
                why: rule.why,
                advice: rule.advice,
                severity: rule.severity,
                recyclable: rule.recyclable,
                size,
                count,
                items: items.into_iter().map(|(size, node)| HogItem { node, path: t.path(node), size }).collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}
