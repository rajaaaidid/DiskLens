//! Binary layout payload sent to the UI (avoids JSON for tens of thousands of blocks).
//!
//! Little-endian. Header (16 bytes): magic `DLLY`, block count u32, names byte length u32, root id u32.
//! Then `count` blocks of 48 bytes:
//!
//! | off | type | field |
//! |-----|------|-------|
//! | 0   | f64  | size (bytes represented) |
//! | 8   | f32×4| x, y, w, h |
//! | 24  | f32  | header height |
//! | 28  | i32  | parent block index (-1 = top level) |
//! | 32  | u32  | node id |
//! | 36  | u32  | aggregate item count |
//! | 40  | u16  | depth |
//! | 42  | u8   | category |
//! | 43  | u8   | flags (see `BF_*`) |
//! | 44  | u32  | files in subtree |
//!
//! Then the names, UTF-8, one per block, separated by NUL (empty for aggregates).

use disklens_core::category::{category_of, Category};
use disklens_core::layout::Block;
use disklens_core::tree::{flags, NodeId, Tree};

pub const BLOCK_BYTES: usize = 48;
pub const BF_AGGREGATE: u8 = 1;
pub const BF_DIR: u8 = 2;
pub const BF_LINK: u8 = 4;
pub const BF_INACCESSIBLE: u8 = 8;
/// Folder whose contents are still being scanned.
pub const BF_PENDING: u8 = 16;

pub fn encode(t: &Tree, root: NodeId, blocks: &[Block], ext_cat: &[u8], dominant: Option<&[u8]>) -> Vec<u8> {
    let mut names = String::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            names.push('\0');
        }
        if !b.aggregate {
            names.push_str(t.name(b.node));
        }
    }
    let mut out = Vec::with_capacity(16 + blocks.len() * BLOCK_BYTES + names.len());
    out.extend_from_slice(b"DLLY");
    out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&root.to_le_bytes());
    for b in blocks {
        let id = b.node;
        let is_dir = t.is_dir(id);
        let category = if b.aggregate {
            Category::Other as u8
        } else if is_dir && b.header > 0.0 {
            Category::Folder as u8
        } else if is_dir {
            dominant.and_then(|d| d.get(id as usize).copied()).unwrap_or(Category::Folder as u8)
        } else {
            category_of(t, ext_cat, id) as u8
        };
        let mut f = 0u8;
        if b.aggregate {
            f |= BF_AGGREGATE;
        } else {
            if is_dir {
                f |= BF_DIR;
                if !t.has_flag(id, flags::SCANNED) && !t.has_flag(id, flags::LINK) {
                    f |= BF_PENDING;
                }
            }
            if t.has_flag(id, flags::LINK) {
                f |= BF_LINK;
            }
            if t.has_flag(id, flags::INACCESSIBLE) {
                f |= BF_INACCESSIBLE;
            }
        }
        out.extend_from_slice(&(b.size as f64).to_le_bytes());
        for v in [b.x, b.y, b.w, b.h, b.header] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&b.parent.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&b.agg_count.to_le_bytes());
        out.extend_from_slice(&b.depth.to_le_bytes());
        out.push(category);
        out.push(f);
        let files = if b.aggregate { b.agg_count } else { t.files[id as usize] };
        out.extend_from_slice(&files.to_le_bytes());
    }
    out.extend_from_slice(names.as_bytes());
    out
}
