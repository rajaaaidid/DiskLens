//! Export a scanned tree (or any subtree) to CSV or JSON, streamed to any writer.
//!
//! Items are written depth-first, largest first. Items sent to the Recycle Bin are left out. Hard-link
//! duplicates and links are included but flagged, and carry 0 bytes, exactly as they are counted.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::category::{category_of, ext_categories};
use crate::tree::{flags, NodeId, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Json,
}

impl std::str::FromStr for Format {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Ok(Format::Csv),
            "json" => Ok(Format::Json),
            _ => Err(format!("unknown format '{s}' (csv, json)")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub format: Format,
    /// Subtree to export (0 = everything scanned).
    pub root: NodeId,
    /// Leave files out (folders only).
    pub folders_only: bool,
    /// Levels below `root` to include (`None` = all).
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub items: u64,
}

/// Unix-epoch ISO-8601 (UTC, seconds) from a Windows FILETIME; empty for 0.
pub fn iso8601(filetime: u64) -> String {
    if filetime == 0 {
        return String::new();
    }
    let secs = (filetime / 10_000_000) as i64 - 11_644_473_600;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn kind_flags(t: &Tree, id: NodeId) -> String {
    let mut v = Vec::new();
    for (f, name) in [(flags::LINK, "link"), (flags::HARDLINK_DUP, "hardlink"), (flags::INACCESSIBLE, "inaccessible"), (flags::REPARSE_OTHER, "reparse")] {
        if t.has_flag(id, f) {
            v.push(name);
        }
    }
    v.join("|")
}

/// Children worth exporting, largest first.
fn kids(t: &Tree, id: NodeId, folders_only: bool) -> Vec<NodeId> {
    let mut k: Vec<NodeId> = t.children(id).filter(|&c| !t.has_flag(c, flags::DELETED) && (!folders_only || t.is_dir(c))).collect();
    k.sort_by(|a, b| t.size[*b as usize].cmp(&t.size[*a as usize]).then_with(|| t.name(*a).cmp(t.name(*b))));
    k
}

pub fn export(t: &Tree, o: Options, w: &mut impl Write) -> io::Result<Stats> {
    if o.root as usize >= t.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no such node"));
    }
    match o.format {
        Format::Csv => export_csv(t, o, w),
        Format::Json => export_json(t, o, w),
    }
}

fn export_csv(t: &Tree, o: Options, w: &mut impl Write) -> io::Result<Stats> {
    let ext_cat = ext_categories(t);
    writeln!(w, "path,type,size,size_on_disk,files,folders,modified,extension,category,flags")?;
    let mut stats = Stats::default();
    // Explicit stack: (node, depth). Children pushed in reverse so the largest comes out first.
    let mut stack = vec![(o.root, 0u32)];
    while let Some((id, depth)) = stack.pop() {
        let i = id as usize;
        let dir = t.is_dir(id);
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{:?},{}",
            csv_field(&t.path(id)),
            if dir { "folder" } else { "file" },
            t.size[i],
            t.alloc[i],
            t.files[i], // 1 for a counted file, 0 for hard-link duplicates and links

            if dir { t.dirs[i] } else { 0 },
            iso8601(t.mtime[i]),
            csv_field(t.ext_name(id)),
            category_of(t, &ext_cat, id),
            kind_flags(t, id),
        )?;
        stats.items += 1;
        if dir && o.max_depth.is_none_or(|m| depth < m) {
            for c in kids(t, id, o.folders_only).into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    Ok(stats)
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn export_json(t: &Tree, o: Options, w: &mut impl Write) -> io::Result<Stats> {
    let mut stats = Stats::default();
    write!(w, "{{\"tool\":\"HaveSpace {}\",\"root\":{},\"tree\":", env!("CARGO_PKG_VERSION"), json_str(&t.path(o.root)))?;
    // Iterative pre-order with explicit closing, so deep trees can't overflow the stack.
    enum Step {
        Open(NodeId, u32, bool),
        Close,
    }
    let mut stack = vec![Step::Open(o.root, 0, true)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Close => write!(w, "]}}")?,
            Step::Open(id, depth, first) => {
                let i = id as usize;
                let dir = t.is_dir(id);
                if !first {
                    write!(w, ",")?;
                }
                write!(w, "{{\"name\":{},\"type\":\"{}\",\"size\":{},\"sizeOnDisk\":{}", json_str(t.name(id)), if dir { "folder" } else { "file" }, t.size[i], t.alloc[i])?;
                if dir {
                    write!(w, ",\"files\":{},\"folders\":{}", t.files[i], t.dirs[i])?;
                }
                let m = iso8601(t.mtime[i]);
                if !m.is_empty() {
                    write!(w, ",\"modified\":\"{m}\"")?;
                }
                let fl = kind_flags(t, id);
                if !fl.is_empty() {
                    write!(w, ",\"flags\":\"{fl}\"")?;
                }
                stats.items += 1;
                let children = if dir { kids(t, id, o.folders_only) } else { Vec::new() };
                if children.is_empty() {
                    write!(w, "}}")?;
                } else if o.max_depth.is_some_and(|m| depth >= m) {
                    write!(w, ",\"truncated\":true}}")?;
                } else {
                    write!(w, ",\"children\":[")?;
                    stack.push(Step::Close);
                    for (k, c) in children.iter().enumerate().rev() {
                        stack.push(Step::Open(*c, depth + 1, k == 0));
                    }
                }
            }
        }
    }
    writeln!(w, "}}")?;
    Ok(stats)
}
