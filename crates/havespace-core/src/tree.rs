//! Compact arena tree: struct-of-arrays indexed by `NodeId` (u32), names interned into one buffer.
//!
//! Children of a folder are stored contiguously (`first_child .. first_child + child_count`), because a
//! folder's entries always arrive as one batch from the scanner. Sizes on folders are subtree totals and
//! are kept up to date incrementally as batches arrive, so a partially scanned tree is always consistent.

use std::collections::HashMap;

pub type NodeId = u32;
pub const NONE: NodeId = u32::MAX;
pub const ROOT: NodeId = 0;

/// Node flag bits.
pub mod flags {
    pub const DIR: u8 = 1 << 0;
    /// Symlink or junction (mount point): recorded but never followed; contributes 0 bytes.
    pub const LINK: u8 = 1 << 1;
    /// Folder could not be opened (access denied etc.).
    pub const INACCESSIBLE: u8 = 1 << 2;
    /// Extra name of an already-counted hard-linked file; contributes 0 bytes.
    pub const HARDLINK_DUP: u8 = 1 << 3;
    /// Sent to the Recycle Bin from HaveSpace; excluded from totals.
    pub const DELETED: u8 = 1 << 4;
    /// Folder enumeration has finished (for streaming partial results).
    pub const SCANNED: u8 = 1 << 5;
    /// Cloud placeholder / other non-link reparse point (e.g. OneDrive).
    pub const REPARSE_OTHER: u8 = 1 << 6;
}

/// One entry as produced by an enumerator, before it is placed in the tree.
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: String,
    pub is_dir: bool,
    /// Logical size (end of file). 0 for folders.
    pub size: u64,
    /// Allocated size on disk. 0 for folders.
    pub alloc: u64,
    /// Last write time as Windows FILETIME (100 ns since 1601).
    pub mtime: u64,
    pub flags: u8,
    /// File identity for hard-link dedup (volume-unique). `None` if unknown.
    pub file_id: Option<u128>,
}

#[derive(Default)]
pub struct Tree {
    pub parent: Vec<NodeId>,
    pub first_child: Vec<NodeId>,
    pub child_count: Vec<u32>,
    name_off: Vec<u32>,
    name_len: Vec<u16>,
    /// Logical bytes (subtree total for folders).
    pub size: Vec<u64>,
    /// Bytes on disk (subtree total for folders).
    pub alloc: Vec<u64>,
    /// Files in subtree (1 for a counted file, 0 for a hard-link duplicate).
    pub files: Vec<u32>,
    /// Folders in subtree, excluding self.
    pub dirs: Vec<u32>,
    pub mtime: Vec<u64>,
    pub ext: Vec<u16>,
    pub flags: Vec<u8>,
    names: String,
    ext_names: Vec<String>,
    ext_lookup: HashMap<String, u16>,
}

impl Tree {
    /// New tree whose root is `root_name` (e.g. `C:\`).
    pub fn new(root_name: &str) -> Self {
        let mut t = Tree::default();
        t.ext_names.push(String::new()); // ext 0 = none
        t.push_node(NONE, root_name, flags::DIR, 0, 0, 0);
        t
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    fn intern_ext(&mut self, name: &str, is_dir: bool) -> u16 {
        if is_dir {
            return 0;
        }
        let Some(dot) = name.rfind('.') else { return 0 };
        if dot == 0 || dot + 1 == name.len() || name.len() - dot > 12 {
            return 0;
        }
        let ext = name[dot + 1..].to_ascii_lowercase();
        if let Some(&id) = self.ext_lookup.get(&ext) {
            return id;
        }
        if self.ext_names.len() >= u16::MAX as usize {
            return 0;
        }
        let id = self.ext_names.len() as u16;
        self.ext_names.push(ext.clone());
        self.ext_lookup.insert(ext, id);
        id
    }

    fn push_node(&mut self, parent: NodeId, name: &str, fl: u8, size: u64, alloc: u64, mtime: u64) -> NodeId {
        let id = self.parent.len() as NodeId;
        let is_dir = fl & flags::DIR != 0;
        let ext = self.intern_ext(name, is_dir);
        self.parent.push(parent);
        self.first_child.push(NONE);
        self.child_count.push(0);
        self.name_off.push(self.names.len() as u32);
        let n = name.len().min(u16::MAX as usize);
        self.name_len.push(n as u16);
        self.names.push_str(&name[..n]);
        let counted = !is_dir && fl & (flags::HARDLINK_DUP | flags::LINK) == 0;
        self.size.push(if counted { size } else { 0 });
        self.alloc.push(if counted { alloc } else { 0 });
        self.files.push(if counted { 1 } else { 0 });
        self.dirs.push(0);
        self.mtime.push(mtime);
        self.ext.push(ext);
        self.flags.push(fl);
        id
    }

    /// Append all entries of folder `dir` as its children and roll their totals up the ancestor chain.
    /// Returns the id of the first child (children are contiguous). Must be called at most once per folder.
    pub fn add_children(&mut self, dir: NodeId, entries: &[RawEntry]) -> NodeId {
        debug_assert_eq!(self.first_child[dir as usize], NONE, "children added twice");
        let first = self.parent.len() as NodeId;
        let (mut size, mut alloc, mut files, mut dirs) = (0u64, 0u64, 0u32, 0u32);
        for e in entries {
            let fl = e.flags | if e.is_dir { flags::DIR } else { 0 };
            let id = self.push_node(dir, &e.name, fl, e.size, e.alloc, e.mtime);
            size += self.size[id as usize];
            alloc += self.alloc[id as usize];
            files += self.files[id as usize];
            if e.is_dir {
                dirs += 1;
            }
        }
        self.first_child[dir as usize] = if entries.is_empty() { NONE } else { first };
        self.child_count[dir as usize] = entries.len() as u32;
        self.flags[dir as usize] |= flags::SCANNED;
        self.propagate(dir, size as i64, alloc as i64, files as i64, dirs as i64);
        first
    }

    /// Add deltas to `node` and every ancestor.
    fn propagate(&mut self, node: NodeId, size: i64, alloc: i64, files: i64, dirs: i64) {
        let mut cur = node;
        while cur != NONE {
            let i = cur as usize;
            self.size[i] = (self.size[i] as i64 + size) as u64;
            self.alloc[i] = (self.alloc[i] as i64 + alloc) as u64;
            self.files[i] = (self.files[i] as i64 + files) as u32;
            self.dirs[i] = (self.dirs[i] as i64 + dirs) as u32;
            cur = self.parent[i];
        }
    }

    pub fn mark(&mut self, node: NodeId, f: u8) {
        self.flags[node as usize] |= f;
    }

    /// Remove a subtree from all totals (after it was sent to the Recycle Bin). No rescan needed.
    pub fn mark_deleted(&mut self, node: NodeId) {
        let i = node as usize;
        if self.flags[i] & flags::DELETED != 0 {
            return;
        }
        let (s, a, f) = (self.size[i] as i64, self.alloc[i] as i64, self.files[i] as i64);
        let d = self.dirs[i] as i64 + if self.is_dir(node) { 1 } else { 0 };
        let p = self.parent[i];
        if p != NONE {
            self.propagate(p, -s, -a, -f, -d);
        }
        self.flags[i] |= flags::DELETED;
    }

    pub fn name(&self, id: NodeId) -> &str {
        let i = id as usize;
        let o = self.name_off[i] as usize;
        &self.names[o..o + self.name_len[i] as usize]
    }

    pub fn ext_name(&self, id: NodeId) -> &str {
        &self.ext_names[self.ext[id as usize] as usize]
    }

    pub fn ext_names(&self) -> &[String] {
        &self.ext_names
    }

    pub fn is_dir(&self, id: NodeId) -> bool {
        self.flags[id as usize] & flags::DIR != 0
    }

    pub fn has_flag(&self, id: NodeId, f: u8) -> bool {
        self.flags[id as usize] & f != 0
    }

    /// Children (including deleted ones; callers filter with `has_flag(DELETED)` if needed).
    pub fn children(&self, id: NodeId) -> std::ops::Range<NodeId> {
        let i = id as usize;
        let f = self.first_child[i];
        if f == NONE {
            0..0
        } else {
            f..f + self.child_count[i]
        }
    }

    /// Full path of a node, e.g. `C:\Users\me\file.txt`.
    pub fn path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NONE {
            parts.push(self.name(cur));
            cur = self.parent[cur as usize];
        }
        parts.reverse();
        let mut s = String::new();
        for (k, p) in parts.iter().enumerate() {
            if k > 0 && !s.ends_with('\\') {
                s.push('\\');
            }
            s.push_str(p);
        }
        s
    }

    /// Find a direct child by name (case-insensitive, as on Windows).
    pub fn child_by_name(&self, id: NodeId, name: &str) -> Option<NodeId> {
        self.children(id).find(|&c| self.name(c).eq_ignore_ascii_case(name))
    }

    /// Approximate heap memory used by the tree.
    pub fn heap_bytes(&self) -> usize {
        let per_node = 4 + 4 + 4 + 4 + 2 + 8 + 8 + 4 + 4 + 8 + 2 + 1;
        self.parent.capacity() * per_node
            + self.names.capacity()
            + self.ext_names.iter().map(|s| s.capacity() + 24).sum::<usize>()
            + self.ext_lookup.capacity() * 40
    }

    /// `true` for every node that is neither deleted nor inside a deleted folder. One forward pass
    /// (parents always have smaller ids than their children).
    pub fn live_mask(&self) -> Vec<bool> {
        let mut live = vec![true; self.len()];
        for i in 0..self.len() {
            let p = self.parent[i];
            live[i] = self.flags[i] & flags::DELETED == 0 && (p == NONE || live[p as usize]);
        }
        live
    }

    /// Release growth slack in the columns once a scan is complete.
    pub fn shrink_to_fit(&mut self) {
        self.parent.shrink_to_fit();
        self.first_child.shrink_to_fit();
        self.child_count.shrink_to_fit();
        self.name_off.shrink_to_fit();
        self.name_len.shrink_to_fit();
        self.size.shrink_to_fit();
        self.alloc.shrink_to_fit();
        self.files.shrink_to_fit();
        self.dirs.shrink_to_fit();
        self.mtime.shrink_to_fit();
        self.ext.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.names.shrink_to_fit();
    }

    /// Recompute every folder total from the leaves (used by tests to verify incremental propagation,
    /// and after loading a cache).
    pub fn recompute_totals(&mut self) {
        let n = self.len();
        // Children always have larger ids than their parent, so a reverse sweep is a post-order.
        for i in 0..n {
            if self.flags[i] & flags::DIR != 0 {
                self.size[i] = 0;
                self.alloc[i] = 0;
                self.files[i] = 0;
                self.dirs[i] = 0;
            }
        }
        for i in (1..n).rev() {
            if self.flags[i] & flags::DELETED != 0 {
                continue;
            }
            let p = self.parent[i] as usize;
            self.size[p] += self.size[i];
            self.alloc[p] += self.alloc[i];
            self.files[p] += self.files[i];
            self.dirs[p] += self.dirs[i] + if self.flags[i] & flags::DIR != 0 { 1 } else { 0 };
        }
    }
}


// ---------------------------------------------------------------------------------------------
// Snapshot: the whole tree as one byte buffer (columns written as-is, little-endian), used by the
// scan cache. Decoding validates every length and index, so a damaged file is an error, not a panic.

const SNAPSHOT_MAGIC: &[u8; 8] = b"DLTREE01";

fn put_u32s(out: &mut Vec<u8>, v: &[u32]) {
    out.reserve(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}
fn put_u64s(out: &mut Vec<u8>, v: &[u64]) {
    out.reserve(v.len() * 8);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}
fn put_u16s(out: &mut Vec<u8>, v: &[u16]) {
    out.reserve(v.len() * 2);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> std::io::Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| bad("truncated snapshot"))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u32s(&mut self, n: usize) -> std::io::Result<Vec<u32>> {
        Ok(self.take(n.checked_mul(4).ok_or_else(|| bad("size overflow"))?)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
    }
    fn u64s(&mut self, n: usize) -> std::io::Result<Vec<u64>> {
        Ok(self.take(n.checked_mul(8).ok_or_else(|| bad("size overflow"))?)?.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
    }
    fn u16s(&mut self, n: usize) -> std::io::Result<Vec<u16>> {
        Ok(self.take(n.checked_mul(2).ok_or_else(|| bad("size overflow"))?)?.chunks_exact(2).map(|c| u16::from_le_bytes(c.try_into().unwrap())).collect())
    }
}

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

impl Tree {
    /// Serialize the tree (uncompressed).
    pub fn to_bytes(&self) -> Vec<u8> {
        let n = self.len();
        let mut out = Vec::with_capacity(16 + n * 53 + self.names.len());
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        put_u32s(&mut out, &self.parent);
        put_u32s(&mut out, &self.first_child);
        put_u32s(&mut out, &self.child_count);
        put_u32s(&mut out, &self.name_off);
        put_u16s(&mut out, &self.name_len);
        put_u64s(&mut out, &self.size);
        put_u64s(&mut out, &self.alloc);
        put_u32s(&mut out, &self.files);
        put_u32s(&mut out, &self.dirs);
        put_u64s(&mut out, &self.mtime);
        put_u16s(&mut out, &self.ext);
        out.extend_from_slice(&self.flags);
        out.extend_from_slice(&(self.names.len() as u32).to_le_bytes());
        out.extend_from_slice(self.names.as_bytes());
        out.extend_from_slice(&(self.ext_names.len() as u32).to_le_bytes());
        for e in &self.ext_names {
            out.extend_from_slice(&(e.len() as u32).to_le_bytes());
            out.extend_from_slice(e.as_bytes());
        }
        out
    }

    /// Rebuild a tree written by [`Tree::to_bytes`], checking that every index and slice is in range.
    pub fn from_bytes(b: &[u8]) -> std::io::Result<Tree> {
        let mut r = Reader { b, pos: 0 };
        if r.take(8)? != SNAPSHOT_MAGIC {
            return Err(bad("not a HaveSpace tree snapshot"));
        }
        let n = r.u32()? as usize;
        if n == 0 || n > b.len() {
            return Err(bad("implausible node count"));
        }
        let mut t = Tree {
            parent: r.u32s(n)?,
            first_child: r.u32s(n)?,
            child_count: r.u32s(n)?,
            name_off: r.u32s(n)?,
            name_len: r.u16s(n)?,
            size: r.u64s(n)?,
            alloc: r.u64s(n)?,
            files: r.u32s(n)?,
            dirs: r.u32s(n)?,
            mtime: r.u64s(n)?,
            ext: r.u16s(n)?,
            flags: r.take(n)?.to_vec(),
            ..Default::default()
        };
        let names_len = r.u32()? as usize;
        t.names = String::from_utf8(r.take(names_len)?.to_vec()).map_err(|_| bad("names are not UTF-8"))?;
        let ext_count = r.u32()? as usize;
        if ext_count == 0 || ext_count > u16::MAX as usize + 1 {
            return Err(bad("implausible extension count"));
        }
        for _ in 0..ext_count {
            let len = r.u32()? as usize;
            let e = String::from_utf8(r.take(len)?.to_vec()).map_err(|_| bad("extension is not UTF-8"))?;
            t.ext_names.push(e);
        }
        for (i, e) in t.ext_names.iter().enumerate().skip(1) {
            t.ext_lookup.insert(e.clone(), i as u16);
        }
        // Validate structure so later indexing can't go out of bounds.
        if t.parent[0] != NONE {
            return Err(bad("root has a parent"));
        }
        for i in 0..n {
            let p = t.parent[i];
            if i > 0 && (p == NONE || p as usize >= i) {
                return Err(bad("parent index out of order"));
            }
            let (o, l) = (t.name_off[i] as usize, t.name_len[i] as usize);
            if o + l > t.names.len() || !t.names.is_char_boundary(o) || !t.names.is_char_boundary(o + l) {
                return Err(bad("name out of range"));
            }
            if t.ext[i] as usize >= t.ext_names.len() {
                return Err(bad("extension index out of range"));
            }
            let (f, c) = (t.first_child[i], t.child_count[i] as usize);
            if f != NONE && (f as usize <= i || f as usize + c > n) {
                return Err(bad("children out of range"));
            }
        }
        Ok(t)
    }
}