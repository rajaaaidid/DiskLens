//! File categories (used for treemap tint and the insights breakdown).
//!
//! The numeric values are shared with the UI (`app/ui/src/data/categories.ts`), so keep the order.

use serde::Serialize;

use crate::tree::{flags, NodeId, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Category {
    Folder = 0,
    Video = 1,
    Image = 2,
    Audio = 3,
    Archive = 4,
    Executable = 5,
    Document = 6,
    Code = 7,
    System = 8,
    Other = 9,
}

pub const COUNT: usize = 10;

impl Category {
    pub const ALL: [Category; COUNT] = [
        Category::Folder,
        Category::Video,
        Category::Image,
        Category::Audio,
        Category::Archive,
        Category::Executable,
        Category::Document,
        Category::Code,
        Category::System,
        Category::Other,
    ];

    pub fn from_u8(v: u8) -> Category {
        Self::ALL.get(v as usize).copied().unwrap_or(Category::Other)
    }

    /// Category for a lower-case extension (without the dot).
    pub fn from_ext(ext: &str) -> Category {
        match ext {
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "mts" | "m2ts" | "flv" | "insv" | "lrv" => Category::Video,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "tif" | "tiff" | "bmp" | "svg" | "ico" | "dds" | "tga" | "exr" | "insp" => {
                Category::Image
            }
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "aiff" => Category::Audio,
            "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "iso" | "cab" | "pak" | "vpk" | "bz2" | "zst" | "tgz" | "img" | "dmg" | "msu" | "psf" | "esd" | "wim" | "ucas" | "utoc" => Category::Archive,
            "exe" | "dll" | "msi" | "msp" | "sys" | "drv" | "ocx" | "msix" | "msixbundle" | "appx" | "appxbundle" | "so" | "node" | "efi" => Category::Executable,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf" | "odt" | "ods" | "epub" | "csv" | "one" => Category::Document,
            "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "rs" | "py" | "pyc" | "c" | "cpp" | "h" | "hpp" | "cs" | "java" | "jar" | "class" | "go" | "json" | "lock" | "map" | "pdb" | "obj" | "lib" | "rlib" | "rmeta" | "o" | "a" | "wasm" | "lua" | "luau" | "html" | "css" | "xml" | "yml" | "yaml" | "toml" | "ps1" | "bat" | "sh" => {
                Category::Code
            }
            "vhdx" | "vhd" | "vmdk" | "qcow2" | "dat" | "log" | "etl" | "evtx" | "mui" | "cat" | "manifest" | "db" | "sqlite" | "ldb" | "edb" | "bin" | "hml" | "tmp" | "blf" | "regtrans-ms" => {
                Category::System
            }
            _ => Category::Other,
        }
    }
}

/// Category per extension id of `t` (index = `t.ext[node]`).
pub fn ext_categories(t: &Tree) -> Vec<u8> {
    t.ext_names().iter().map(|e| Category::from_ext(e) as u8).collect()
}

/// Category of a single node: files by extension, folders as `Folder`.
pub fn category_of(t: &Tree, ext_cat: &[u8], id: NodeId) -> Category {
    if t.is_dir(id) {
        Category::Folder
    } else if t.ext_name(id) == "sys" && is_system_swap_file(t.name(id)) {
        // pagefile.sys and friends are .sys files but not drivers.
        Category::System
    } else {
        Category::from_u8(ext_cat.get(t.ext[id as usize] as usize).copied().unwrap_or(Category::Other as u8))
    }
}

fn is_system_swap_file(name: &str) -> bool {
    ["pagefile.sys", "hiberfil.sys", "swapfile.sys"].iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// For every folder, the category holding the most bytes in its subtree (files: their own category).
/// One post-order pass; temporary memory is `COUNT × 8` bytes per folder.
pub fn dominant_categories(t: &Tree, ext_cat: &[u8]) -> Vec<u8> {
    let n = t.len();
    let mut out = vec![Category::Other as u8; n];
    // Folder index -> slot in the per-category accumulator.
    let mut slot = vec![u32::MAX; n];
    let mut acc: Vec<[u64; COUNT]> = Vec::new();
    for i in 0..n {
        if t.is_dir(i as NodeId) {
            slot[i] = acc.len() as u32;
            acc.push([0; COUNT]);
        }
    }
    // Children have larger ids than parents, so a reverse sweep sees every child before its parent.
    for i in (0..n).rev() {
        let id = i as NodeId;
        if t.has_flag(id, flags::DELETED) {
            continue;
        }
        let p = t.parent[i];
        if t.is_dir(id) {
            let a = acc[slot[i] as usize];
            let (best, _) = a.iter().enumerate().skip(1).max_by_key(|(_, s)| **s).unwrap();
            out[i] = if a.iter().all(|s| *s == 0) { Category::Folder as u8 } else { best as u8 };
            if p != crate::tree::NONE {
                let pa = &mut acc[slot[p as usize] as usize];
                for c in 0..COUNT {
                    pa[c] += a[c];
                }
            }
        } else {
            let c = category_of(t, ext_cat, id) as usize;
            out[i] = c as u8;
            if p != crate::tree::NONE {
                acc[slot[p as usize] as usize][c] += t.size[i];
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryTotal {
    pub category: Category,
    pub size: u64,
    pub files: u64,
}

pub fn category_totals(t: &Tree, ext_cat: &[u8]) -> Vec<CategoryTotal> {
    let mut acc = [(0u64, 0u64); COUNT];
    for id in 0..t.len() as NodeId {
        if t.is_dir(id) || t.has_flag(id, flags::HARDLINK_DUP | flags::LINK | flags::DELETED) {
            continue;
        }
        let c = category_of(t, ext_cat, id) as usize;
        acc[c].0 += t.size[id as usize];
        acc[c].1 += 1;
    }
    let mut v: Vec<CategoryTotal> = acc
        .iter()
        .enumerate()
        .filter(|(_, (_, f))| *f > 0)
        .map(|(i, (s, f))| CategoryTotal { category: Category::from_u8(i as u8), size: *s, files: *f })
        .collect();
    v.sort_by(|a, b| b.size.cmp(&a.size));
    v
}
