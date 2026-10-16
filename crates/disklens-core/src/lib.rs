//! DiskLens scanner engine.
//!
//! - [`tree`]: compact arena of the scanned file system (struct-of-arrays, interned names).
//! - [`scan`]: parallel directory walk and NTFS MFT fast path, with live progress and cancel.
//! - [`layout`]: nested squarified treemap layout.
//! - [`analysis`]: largest files/folders and extension totals.

pub mod analysis;
pub mod category;
pub mod layout;
pub mod scan;
pub mod tree;

pub use scan::{scan, Method, Progress, ScanOptions, ScanResult};
pub use tree::{NodeId, Tree};

/// Human-readable byte count (binary units).
pub fn format_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < U.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", U[u])
    } else {
        format!("{v:.1} {}", U[u])
    }
}
