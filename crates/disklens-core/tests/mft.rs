//! MFT parsing and tree building on synthetic, byte-exact NTFS file records.

use disklens_core::scan::mft::build::{build_tree, merge, ROOT_RECORD};
use disklens_core::scan::mft::record::{apply_fixup, decode_runlist, parse_record};
use disklens_core::scan::Progress;
use disklens_core::tree::{flags, NodeId, Tree, ROOT};

const REC: usize = 1024;
const USN: u16 = 0x0007;

enum Attr<'a> {
    /// modified time, file attributes
    Std(u64, u32),
    /// parent record, parent sequence, namespace, name
    Name(u32, u16, u8, &'a str),
    DataResident(usize),
    /// starting VCN, allocated, real, flags, total allocated
    DataNonResident(u64, u64, u64, u16, u64),
    Reparse(u32),
}

/// Build one 1 KiB FILE record with correct update-sequence fixups.
fn record(seq: u16, flags: u16, base: u32, attrs: &[Attr]) -> Vec<u8> {
    let mut r = vec![0u8; REC];
    r[0..4].copy_from_slice(b"FILE");
    r[4..6].copy_from_slice(&48u16.to_le_bytes()); // USA offset
    r[6..8].copy_from_slice(&3u16.to_le_bytes()); // 1 + 2 sectors
    r[16..18].copy_from_slice(&seq.to_le_bytes());
    r[20..22].copy_from_slice(&56u16.to_le_bytes()); // first attribute
    r[22..24].copy_from_slice(&flags.to_le_bytes());
    r[28..32].copy_from_slice(&(REC as u32).to_le_bytes());
    r[32..40].copy_from_slice(&((base as u64) | if base != 0 { 1u64 << 48 } else { 0 }).to_le_bytes());
    let mut off = 56;
    let mut put_attr = |r: &mut Vec<u8>, ty: u32, non_res: bool, name_len: u8, aflags: u16, body: &[u8], value: bool| {
        // value=true: resident header (24 bytes) + value; else non-resident header (72 bytes) given in body.
        let hdr = if value { 24 } else { 0 };
        let len = (hdr + body.len()).div_ceil(8) * 8;
        r[off..off + 4].copy_from_slice(&ty.to_le_bytes());
        r[off + 4..off + 8].copy_from_slice(&(len as u32).to_le_bytes());
        r[off + 8] = non_res as u8;
        r[off + 9] = name_len;
        r[off + 12..off + 14].copy_from_slice(&aflags.to_le_bytes());
        if value {
            r[off + 16..off + 20].copy_from_slice(&(body.len() as u32).to_le_bytes());
            r[off + 20..off + 22].copy_from_slice(&24u16.to_le_bytes());
            r[off + 24..off + 24 + body.len()].copy_from_slice(body);
        } else {
            r[off + 16..off + body.len()].copy_from_slice(&body[16..]);
        }
        off += len;
    };
    for a in attrs {
        match a {
            Attr::Std(mtime, fattrs) => {
                let mut v = vec![0u8; 48];
                v[8..16].copy_from_slice(&mtime.to_le_bytes());
                v[32..36].copy_from_slice(&fattrs.to_le_bytes());
                put_attr(&mut r, 0x10, false, 0, 0, &v, true);
            }
            Attr::Name(parent, pseq, ns, name) => {
                let units: Vec<u16> = name.encode_utf16().collect();
                let mut v = vec![0u8; 66 + units.len() * 2];
                v[0..8].copy_from_slice(&((*parent as u64) | ((*pseq as u64) << 48)).to_le_bytes());
                v[64] = units.len() as u8;
                v[65] = *ns;
                for (i, u) in units.iter().enumerate() {
                    v[66 + i * 2..68 + i * 2].copy_from_slice(&u.to_le_bytes());
                }
                put_attr(&mut r, 0x30, false, 0, 0, &v, true);
            }
            Attr::DataResident(n) => put_attr(&mut r, 0x80, false, 0, 0, &vec![0xAB; *n], true),
            Attr::DataNonResident(vcn, alloc, real, aflags, total) => {
                let mut v = vec![0u8; 80];
                v[16..24].copy_from_slice(&vcn.to_le_bytes());
                v[32..34].copy_from_slice(&72u16.to_le_bytes()); // runlist offset
                v[40..48].copy_from_slice(&alloc.to_le_bytes());
                v[48..56].copy_from_slice(&real.to_le_bytes());
                v[56..64].copy_from_slice(&real.to_le_bytes());
                v[64..72].copy_from_slice(&total.to_le_bytes());
                v[72] = 0x11; // one run: 1 byte length, 1 byte offset
                v[73] = 1;
                v[74] = 0x20;
                put_attr(&mut r, 0x80, true, 0, *aflags, &v, false);
            }
            Attr::Reparse(tag) => {
                let mut v = vec![0u8; 8];
                v[0..4].copy_from_slice(&tag.to_le_bytes());
                put_attr(&mut r, 0xC0, false, 0, 0, &v, true);
            }
        }
    }
    r[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    r[24..28].copy_from_slice(&((off + 8) as u32).to_le_bytes());
    // Fixups: stash the real sector-end bytes in the USA and stamp the USN over them.
    r[48..50].copy_from_slice(&USN.to_le_bytes());
    for i in 1..3 {
        let end = i * 512 - 2;
        let orig = [r[end], r[end + 1]];
        r[48 + i * 2] = orig[0];
        r[48 + i * 2 + 1] = orig[1];
        r[end..end + 2].copy_from_slice(&USN.to_le_bytes());
    }
    r
}

const IN_USE: u16 = 1;
const DIR: u16 = 3;
const WIN32: u8 = 1;
const DOS: u8 = 2;
const WIN32_AND_DOS: u8 = 3;

fn parsed(rec_no: u32, mut r: Vec<u8>) -> disklens_core::scan::mft::record::Parsed {
    assert!(apply_fixup(&mut r));
    parse_record(&r, rec_no).expect("record should parse")
}

#[test]
fn fixups_restore_sector_ends_and_detect_torn_records() {
    let mut r = record(1, IN_USE, 0, &[Attr::Std(0, 0), Attr::DataResident(700)]);
    // Resident data spans the first sector boundary; after fixup the bytes must be the originals.
    assert!(apply_fixup(&mut r));
    assert_eq!(&r[510..512], &[0xAB, 0xAB]);
    let mut torn = record(1, IN_USE, 0, &[Attr::Std(0, 0)]);
    torn[1022] ^= 0xFF;
    assert!(!apply_fixup(&mut torn));
    let mut not_file = vec![0u8; REC];
    assert!(!apply_fixup(&mut not_file));
}

#[test]
fn parses_names_sizes_and_reparse_tags() {
    let p = parsed(
        40,
        record(9, IN_USE, 0, &[Attr::Std(1234, 0x20), Attr::Name(5, 5, DOS, "PROGRA~1.TXT"), Attr::Name(5, 5, WIN32, "Program notes.txt"), Attr::DataNonResident(0, 8192, 5000, 0, 0)]),
    );
    assert_eq!(p.rec, 40);
    assert_eq!(p.base, 40);
    assert_eq!(p.seq, 9);
    assert!(!p.is_dir);
    assert_eq!(p.mtime, 1234);
    assert_eq!(p.names, vec![(5, 5, "Program notes.txt".to_string())], "DOS 8.3 alias must be skipped");
    assert_eq!(p.data, Some((5000, 8192)));

    let small = parsed(41, record(1, IN_USE, 0, &[Attr::Name(5, 5, WIN32_AND_DOS, "a.txt"), Attr::DataResident(300)]));
    assert_eq!(small.data, Some((300, 0)), "resident data occupies no clusters");

    let sparse = parsed(42, record(1, IN_USE, 0, &[Attr::Name(5, 5, WIN32, "big.vhdx"), Attr::DataNonResident(0, 1 << 40, 1 << 40, 0x8000, 4096)]));
    assert_eq!(sparse.data, Some((1 << 40, 4096)), "sparse files report their real allocation");

    let tail = parsed(43, record(1, IN_USE, 0, &[Attr::DataNonResident(64, 8192, 5000, 0, 0)]));
    assert_eq!(tail.data, None, "only the VCN-0 segment carries sizes");

    let link = parsed(44, record(1, DIR, 0, &[Attr::Std(0, 0x410), Attr::Name(5, 5, WIN32, "junction"), Attr::Reparse(0xA000_0003)]));
    assert!(link.is_dir);
    assert_eq!(link.reparse_tag, 0xA000_0003);

    let ext = parsed(45, record(1, IN_USE, 44, &[]));
    assert_eq!(ext.base, 44);

    let mut unused = record(1, 0, 0, &[Attr::Name(5, 5, WIN32, "deleted.txt")]);
    assert!(apply_fixup(&mut unused));
    assert!(parse_record(&unused, 46).is_none(), "records not in use are ignored");
}

#[test]
fn runlists_decode_with_relative_and_sparse_runs() {
    // 0x21: 1-byte length, 2-byte offset. Second run goes backwards (negative delta), third is sparse.
    let rl = [0x21, 0x10, 0x00, 0x10, 0x21, 0x08, 0x00, 0xF8, 0x01, 0x04, 0x00];
    let runs = decode_runlist(&rl).unwrap();
    assert_eq!(runs, vec![(Some(0x1000), 0x10), (Some(0x1000 - 0x800), 0x08), (None, 4)]);
    assert!(decode_runlist(&[0x21, 0x10]).is_none(), "truncated runlist");
}

/// root(5)
///  ├─ docs/ (30)          ← hard link second name of report.pdf
///  │    └─ report.pdf (31)  10 000 B, names in docs/ and in archive/
///  ├─ archive/ (32)
///  ├─ big.iso (33)          $DATA lives in extension record 34
///  ├─ junction/ (35)        reparse point (mount point): flagged LINK
///  ├─ tiny.txt (36)         resident 100 B
///  └─ orphan.txt (37)       parent ref has a stale sequence → skipped
fn sample_records() -> Vec<disklens_core::scan::mft::record::Parsed> {
    vec![
        parsed(ROOT_RECORD, record(5, DIR, 0, &[Attr::Std(0, 0), Attr::Name(5, 5, WIN32_AND_DOS, ".")])),
        parsed(30, record(1, DIR, 0, &[Attr::Std(0, 0), Attr::Name(5, 5, WIN32, "docs")])),
        parsed(31, record(1, IN_USE, 0, &[Attr::Std(7, 0x20), Attr::Name(30, 1, WIN32, "report.pdf"), Attr::Name(32, 1, WIN32, "report-link.pdf"), Attr::DataNonResident(0, 12288, 10_000, 0, 0)])),
        parsed(32, record(1, DIR, 0, &[Attr::Name(5, 5, WIN32, "archive")])),
        parsed(33, record(1, IN_USE, 0, &[Attr::Name(5, 5, WIN32, "big.iso")])),
        parsed(34, record(1, IN_USE, 33, &[Attr::DataNonResident(0, 1 << 30, 1 << 30, 0, 0)])),
        parsed(35, record(1, DIR, 0, &[Attr::Std(0, 0x410), Attr::Name(5, 5, WIN32, "junction"), Attr::Reparse(0xA000_0003)])),
        parsed(36, record(1, IN_USE, 0, &[Attr::Name(5, 5, WIN32, "tiny.txt"), Attr::DataResident(100)])),
        parsed(37, record(1, IN_USE, 0, &[Attr::Name(30, 99, WIN32, "orphan.txt"), Attr::DataResident(50)])),
    ]
}

fn find(t: &Tree, rel: &str) -> NodeId {
    let mut cur = ROOT;
    for part in rel.split('\\') {
        cur = t.child_by_name(cur, part).unwrap_or_else(|| panic!("missing {rel}"));
    }
    cur
}

#[test]
fn builds_the_tree_with_hard_links_once_and_no_orphans() {
    let mut recs = merge(sample_records(), 64);
    let mut t = Tree::new("C:\\");
    let progress = Progress::default();
    let stats = build_tree(&mut t, &mut recs, &progress);

    assert_eq!(t.size[0], 10_000 + (1 << 30) + 100, "hard link counted once, orphan skipped");
    assert_eq!(t.files[0], 3);
    assert_eq!(t.dirs[0], 3);
    assert_eq!(stats.orphans, 1);
    assert_eq!(stats.hardlink_names, 1);

    // Both names are visible; exactly one is the counted one.
    let a = find(&t, "docs\\report.pdf");
    let b = find(&t, "archive\\report-link.pdf");
    assert_eq!(t.size[a as usize] + t.size[b as usize], 10_000);
    assert!(t.has_flag(a, flags::HARDLINK_DUP) != t.has_flag(b, flags::HARDLINK_DUP));
    assert_eq!(t.alloc[find(&t, "big.iso") as usize], 1 << 30, "size from the extension record");
    assert!(t.has_flag(find(&t, "junction"), flags::LINK));
    assert_eq!(t.alloc[find(&t, "tiny.txt") as usize], 0);
    assert!(t.child_by_name(ROOT, "orphan.txt").is_none());
    assert_eq!(t.mtime[a as usize], 7);

    // Live counters match the tree.
    let s = progress.snapshot();
    assert_eq!(s.files, 3);
    assert_eq!(s.bytes, t.size[0]);
    // Incremental totals agree with a full recompute.
    let before = t.size.clone();
    t.recompute_totals();
    assert_eq!(before, t.size);
}

#[test]
fn records_out_of_order_and_cycles_are_safe() {
    let mut v = sample_records();
    v.reverse();
    // A corrupt directory cycle: 50 -> 51 -> 50, unreachable from the root.
    v.push(parsed(50, record(1, DIR, 0, &[Attr::Name(51, 1, WIN32, "a")])));
    v.push(parsed(51, record(1, DIR, 0, &[Attr::Name(50, 1, WIN32, "b")])));
    let mut recs = merge(v, 64);
    let mut t = Tree::new("C:\\");
    let stats = build_tree(&mut t, &mut recs, &Progress::default());
    assert_eq!(t.size[0], 10_000 + (1 << 30) + 100);
    assert_eq!(stats.orphans, 3);
}
