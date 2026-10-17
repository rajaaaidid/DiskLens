//! Parsing of NTFS file records (pure byte-level code, no I/O, never panics on bad data).
//!
//! Layout references: the NTFS on-disk format as documented by the Linux-NTFS project.

/// What one in-use file record contributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    /// This record's number.
    pub rec: u32,
    /// The base record it belongs to (== `rec` for base records; extension records point elsewhere).
    pub base: u32,
    /// Sequence number of this record (to validate parent references).
    pub seq: u16,
    pub is_dir: bool,
    /// From $STANDARD_INFORMATION (0 if this record has none).
    pub attrs: u32,
    pub mtime: u64,
    pub reparse_tag: u32,
    /// Logical size and size on disk of the unnamed $DATA stream, if this record holds its first segment.
    pub data: Option<(u64, u64)>,
    /// Non-DOS names: (parent record, parent sequence, name).
    pub names: Vec<(u32, u16, String)>,
}

pub const RECORD_IN_USE: u16 = 0x0001;
pub const RECORD_IS_DIR: u16 = 0x0002;

const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;

const NAMESPACE_DOS: u8 = 2;
const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
const ATTR_FLAG_SPARSE: u16 = 0x8000;

#[inline]
fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
#[inline]
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}
#[inline]
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

/// Apply the update-sequence fixups in place (the last two bytes of every 512-byte stride were
/// replaced by the sequence number when the record was written). Returns false if the record is torn
/// or not a FILE record.
pub fn apply_fixup(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let (Some(usa_off), Some(usa_count)) = (u16_at(rec, 4), u16_at(rec, 6)) else { return false };
    let (usa_off, usa_count) = (usa_off as usize, usa_count as usize);
    if usa_count < 2 || usa_off + usa_count * 2 > rec.len() || (usa_count - 1) * 512 > rec.len() {
        return false;
    }
    let check = [rec[usa_off], rec[usa_off + 1]];
    for i in 1..usa_count {
        let end = i * 512 - 2;
        if rec[end..end + 2] != check {
            return false; // torn write
        }
        rec[end] = rec[usa_off + i * 2];
        rec[end + 1] = rec[usa_off + i * 2 + 1];
    }
    true
}

/// Parse a record that already had its fixups applied. Returns `None` for records not in use or
/// malformed ones.
pub fn parse_record(rec: &[u8], rec_no: u32) -> Option<Parsed> {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return None;
    }
    let flags = u16_at(rec, 22)?;
    if flags & RECORD_IN_USE == 0 {
        return None;
    }
    let base_ref = u64_at(rec, 32)? & 0x0000_FFFF_FFFF_FFFF;
    let mut p = Parsed {
        rec: rec_no,
        base: if base_ref == 0 { rec_no } else { base_ref as u32 },
        seq: u16_at(rec, 16)?,
        is_dir: flags & RECORD_IS_DIR != 0,
        ..Default::default()
    };
    let used = (u32_at(rec, 24)? as usize).min(rec.len());
    let mut off = u16_at(rec, 20)? as usize;
    // Bounded walk over the attribute list.
    for _ in 0..256 {
        if off + 16 > used {
            break;
        }
        let ty = u32_at(rec, off)?;
        if ty == ATTR_END {
            break;
        }
        let len = u32_at(rec, off + 4)? as usize;
        if len < 16 || off + len > used {
            break;
        }
        let a = &rec[off..off + len];
        let non_resident = a[8] != 0;
        let name_len = a[9];
        match ty {
            ATTR_STANDARD_INFORMATION if !non_resident => {
                if let Some(v) = resident_value(a) {
                    p.mtime = u64_at(v, 8).unwrap_or(0);
                    p.attrs = u32_at(v, 32).unwrap_or(0);
                }
            }
            ATTR_FILE_NAME if !non_resident => {
                if let Some(v) = resident_value(a) {
                    if let Some(n) = file_name(v) {
                        p.names.push(n);
                    }
                }
            }
            ATTR_DATA if name_len == 0 => {
                if non_resident {
                    // Only the segment starting at VCN 0 carries the stream sizes.
                    if u64_at(a, 16) == Some(0) {
                        let aflags = u16_at(a, 12).unwrap_or(0);
                        let alloc = u64_at(a, 40).unwrap_or(0);
                        let size = u64_at(a, 48).unwrap_or(0);
                        let on_disk = if aflags & (ATTR_FLAG_COMPRESSED | ATTR_FLAG_SPARSE) != 0 { u64_at(a, 64).unwrap_or(alloc) } else { alloc };
                        p.data = Some((size, on_disk));
                    }
                } else if let Some(v) = resident_value(a) {
                    // Small files live inside the MFT record itself: no clusters of their own.
                    p.data = Some((v.len() as u64, 0));
                }
            }
            ATTR_REPARSE_POINT => {
                if !non_resident {
                    if let Some(v) = resident_value(a) {
                        p.reparse_tag = u32_at(v, 0).unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
        off += len;
    }
    Some(p)
}

fn resident_value(a: &[u8]) -> Option<&[u8]> {
    let vlen = u32_at(a, 16)? as usize;
    let voff = u16_at(a, 20)? as usize;
    a.get(voff..voff.checked_add(vlen)?)
}

fn file_name(v: &[u8]) -> Option<(u32, u16, String)> {
    let parent = u64_at(v, 0)?;
    let nlen = *v.get(64)? as usize;
    let ns = *v.get(65)?;
    if ns == NAMESPACE_DOS {
        return None; // 8.3 alias of a name we get in the Win32 namespace
    }
    let raw = v.get(66..66 + nlen * 2)?;
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(((parent & 0x0000_FFFF_FFFF_FFFF) as u32, (parent >> 48) as u16, String::from_utf16_lossy(&units)))
}

/// Find the unnamed $DATA attribute's runlist in a (fixed-up) record: returns
/// (runlist bytes, starting VCN, last VCN).
pub fn data_runlist(rec: &[u8]) -> Option<(&[u8], u64, u64)> {
    let used = (u32_at(rec, 24)? as usize).min(rec.len());
    let mut off = u16_at(rec, 20)? as usize;
    for _ in 0..256 {
        if off + 16 > used {
            return None;
        }
        let ty = u32_at(rec, off)?;
        if ty == ATTR_END {
            return None;
        }
        let len = u32_at(rec, off + 4)? as usize;
        if len < 16 || off + len > used {
            return None;
        }
        let a = &rec[off..off + len];
        if ty == ATTR_DATA && a[8] != 0 && a[9] == 0 {
            let start = u64_at(a, 16)?;
            let last = u64_at(a, 24)?;
            let ro = u16_at(a, 32)? as usize;
            return Some((a.get(ro..)?, start, last));
        }
        off += len;
    }
    None
}

/// Decode a runlist into (LCN, cluster count) extents. Sparse runs are returned with LCN `None`.
pub fn decode_runlist(mut r: &[u8]) -> Option<Vec<(Option<u64>, u64)>> {
    let mut out = Vec::new();
    let mut lcn: i64 = 0;
    while let Some(&h) = r.first() {
        if h == 0 {
            break;
        }
        let (ls, os) = ((h & 0x0F) as usize, (h >> 4) as usize);
        if ls == 0 || ls > 8 || os > 8 || r.len() < 1 + ls + os {
            return None;
        }
        let mut len = 0u64;
        for i in 0..ls {
            len |= (r[1 + i] as u64) << (8 * i);
        }
        if os == 0 {
            out.push((None, len));
        } else {
            let mut delta: i64 = 0;
            for i in 0..os {
                delta |= (r[1 + ls + i] as i64) << (8 * i);
            }
            // Sign-extend.
            let shift = 64 - 8 * os as u32;
            delta = (delta << shift) >> shift;
            lcn = lcn.checked_add(delta)?;
            if lcn < 0 {
                return None;
            }
            out.push((Some(lcn as u64), len));
        }
        r = &r[1 + ls + os..];
    }
    Some(out)
}
