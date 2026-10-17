//! NTFS fast path: read the Master File Table directly instead of walking directories.
//!
//! 1. Open the raw volume (`\\.\C:`), which needs administrator rights, and ask NTFS where the MFT is
//!    (`FSCTL_GET_NTFS_VOLUME_DATA`).
//! 2. Read file record 0 (`$MFT` itself) and decode its unnamed `$DATA` runlist: the MFT's own extents.
//! 3. A reader thread streams those extents in large aligned chunks; worker threads apply fixups and
//!    parse each record (names, parent references, sizes, reparse tags).
//! 4. Extension records are folded into their base records and the tree is built breadth-first from
//!    the root directory (record 5). A file with several names (hard links) is counted once.
//!
//! Anything unexpected (not NTFS, not a whole drive, no admin rights, an MFT so fragmented that its own
//! extents spill into an attribute list) returns an error and `Method::Auto` falls back to the walk.

pub mod build;
pub mod record;

use std::io;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::Progress;
use crate::tree::Tree;

#[cfg(windows)]
pub use win::scan_volume;

#[cfg(not(windows))]
pub fn scan_volume(_root: &Path, _progress: &Progress, _tree: &Arc<RwLock<Tree>>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the MFT fast path is Windows-only"))
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::FileExt;
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    use crossbeam_channel::bounded;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
    use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use super::build::{build_tree, Records};
    use super::record::{apply_fixup, data_runlist, decode_runlist, parse_record, Batch};
    use crate::scan::root_display_name;

    /// Bytes per read request. Large sequential reads keep NVMe/SATA drives at full speed.
    const CHUNK: usize = 4 << 20;

    fn unsupported(msg: impl Into<String>) -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, msg.into())
    }

    fn filesystem_name(root: &str) -> Option<String> {
        let w: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let mut fs = [0u16; 32];
        let ok = unsafe {
            GetVolumeInformationW(w.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), fs.len() as u32)
        };
        (ok != 0).then(|| String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(0)]))
    }

    fn volume_data(vol: &File) -> io::Result<NTFS_VOLUME_DATA_BUFFER> {
        let mut data: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
        let mut ret = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                vol.as_raw_handle() as _,
                FSCTL_GET_NTFS_VOLUME_DATA,
                std::ptr::null(),
                0,
                (&mut data as *mut NTFS_VOLUME_DATA_BUFFER).cast(),
                std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(data)
    }

    /// Read exactly `buf.len()` bytes at `offset` (volume reads must be sector-aligned; ours are).
    fn read_at(vol: &File, buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        let mut done = 0;
        while done < buf.len() {
            let n = vol.seek_read(&mut buf[done..], offset)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read from volume"));
            }
            done += n;
            offset += n as u64;
        }
        Ok(())
    }

    pub fn scan_volume(root: &Path, progress: &Progress, tree: &Arc<RwLock<Tree>>) -> io::Result<()> {
        let start = Instant::now();
        let root_s = root_display_name(root);
        if !(root_s.len() == 3 && root_s.ends_with(":\\")) {
            return Err(unsupported("the MFT is only read for whole drives; folders use the directory walk"));
        }
        match filesystem_name(&root_s).as_deref() {
            Some("NTFS") => {}
            Some(other) => return Err(unsupported(format!("{other} has no MFT (only NTFS does)"))),
            None => return Err(unsupported("could not read the file system type")),
        }
        let vol = OpenOptions::new().read(true).open(format!(r"\\.\{}", &root_s[..2])).map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                io::Error::new(io::ErrorKind::PermissionDenied, "reading the MFT needs administrator rights")
            } else {
                e
            }
        })?;
        let vd = volume_data(&vol)?;
        let rec_size = vd.BytesPerFileRecordSegment as usize;
        let cluster = vd.BytesPerCluster as u64;
        if !(1024..=65536).contains(&rec_size) || rec_size % 512 != 0 || cluster == 0 {
            return Err(unsupported(format!("unexpected NTFS geometry (record {rec_size} B, cluster {cluster} B)")));
        }
        let total_records = (vd.MftValidDataLength as u64 / rec_size as u64) as usize;

        // Record 0 describes the MFT's own extents.
        let first_len = (rec_size as u64).max(cluster).div_ceil(cluster) * cluster;
        let mut rec0 = vec![0u8; first_len as usize];
        read_at(&vol, &mut rec0, vd.MftStartLcn as u64 * cluster)?;
        let rec0 = &mut rec0[..rec_size];
        if !apply_fixup(rec0) {
            return Err(unsupported("MFT record 0 is damaged"));
        }
        let (runlist, start_vcn, last_vcn) = data_runlist(rec0).ok_or_else(|| unsupported("no $DATA runlist in MFT record 0"))?;
        let extents = decode_runlist(runlist).ok_or_else(|| unsupported("invalid MFT runlist"))?;
        let needed_clusters = (vd.MftValidDataLength as u64).div_ceil(cluster);
        if start_vcn != 0 || last_vcn + 1 < needed_clusters {
            return Err(unsupported("the MFT is too fragmented to read directly (its extents continue in an attribute list)"));
        }

        // Split the extents into read jobs of up to CHUNK bytes: (byte offset, first record, records).
        let mut jobs: Vec<(Option<u64>, usize, usize)> = Vec::new();
        let mut rec_no = 0usize;
        let per_chunk = (CHUNK / rec_size).max(1);
        for &(lcn, len) in &extents {
            let mut recs_left = ((len * cluster) as usize / rec_size).min(total_records.saturating_sub(rec_no));
            let mut off = 0u64;
            while recs_left > 0 {
                let n = recs_left.min(per_chunk);
                jobs.push((lcn.map(|l| l * cluster + off), rec_no, n));
                rec_no += n;
                recs_left -= n;
                off += (n * rec_size) as u64;
            }
        }
        let extent_count = extents.len();

        // Reader threads (each with its own volume handle, so reads overlap) -> parsers -> collector.
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let readers = 4usize.min(jobs.len().max(1));
        let parsers = cores.clamp(2, 16);
        let next_job = std::sync::atomic::AtomicUsize::new(0);
        let (chunk_tx, chunk_rx) = bounded::<(usize, Vec<u8>)>(parsers * 2);
        let (out_tx, out_rx) = bounded::<Batch>(parsers * 2);
        let read_err: std::sync::Mutex<Option<io::Error>> = std::sync::Mutex::new(None);
        let volume_path = format!(r"\\.\{}", &root_s[..2]);
        let read_done_ms = std::sync::atomic::AtomicU64::new(0);

        let batches: Vec<Batch> = std::thread::scope(|s| {
            for _ in 0..readers {
                let (jobs, next_job, read_err, chunk_tx, volume_path, read_done_ms) = (&jobs, &next_job, &read_err, chunk_tx.clone(), &volume_path, &read_done_ms);
                s.spawn(move || {
                    let vol = match OpenOptions::new().read(true).open(volume_path) {
                        Ok(v) => v,
                        Err(e) => {
                            *read_err.lock().unwrap() = Some(e);
                            return;
                        }
                    };
                    loop {
                        let j = next_job.fetch_add(1, Ordering::Relaxed);
                        if j >= jobs.len() || progress.is_cancelled() {
                            break;
                        }
                        let (offset, first, n) = jobs[j];
                        let read_len = (n * rec_size).div_ceil(cluster as usize) * cluster as usize;
                        let mut buf = vec![0u8; read_len];
                        if let Some(o) = offset {
                            if let Err(e) = read_at(&vol, &mut buf, o) {
                                *read_err.lock().unwrap() = Some(e);
                                break;
                            }
                        }
                        buf.truncate(n * rec_size);
                        if chunk_tx.send((first, buf)).is_err() {
                            break;
                        }
                    }
                    read_done_ms.fetch_max(start.elapsed().as_millis() as u64, Ordering::Relaxed);
                });
            }
            drop(chunk_tx);
            for _ in 0..parsers {
                let rx = chunk_rx.clone();
                let tx = out_tx.clone();
                s.spawn(move || {
                    for (first, mut buf) in rx.iter() {
                        let mut out = Batch::with_capacity(buf.len() / rec_size);
                        let (mut files, mut dirs) = (0u64, 0u64);
                        for (i, rec) in buf.chunks_exact_mut(rec_size).enumerate() {
                            if apply_fixup(rec) && parse_record(rec, (first + i) as u32, &mut out) {
                                let r = out.recs.last().unwrap();
                                if r.base == r.rec && r.name_count > 0 {
                                    if r.is_dir {
                                        dirs += 1;
                                    } else {
                                        files += 1;
                                    }
                                }
                            }
                        }
                        out.recs.shrink_to_fit();
                        out.names.shrink_to_fit();
                        out.text.shrink_to_fit();
                        // Rough live counters while reading; exact totals come from the tree build.
                        progress.dirs.fetch_add(dirs, Ordering::Relaxed);
                        progress.files.fetch_add(files, Ordering::Relaxed);
                        if tx.send(out).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(chunk_rx);
            drop(out_tx);
            out_rx.iter().collect()
        });
        if let Some(e) = read_err.into_inner().unwrap() {
            return Err(e);
        }
        if progress.is_cancelled() {
            return Ok(());
        }
        let parse_ms = start.elapsed().as_millis();

        let recs = Records::new(batches, total_records);
        let records_mib = recs.heap_bytes() >> 20;
        // Reset the rough counters; the build reports exact numbers.
        progress.files.store(0, Ordering::Relaxed);
        progress.dirs.store(0, Ordering::Relaxed);
        progress.bytes.store(0, Ordering::Relaxed);
        let stats = {
            let mut t = tree.write().unwrap();
            let stats = build_tree(&mut t, &recs, progress);
            t.shrink_to_fit();
            stats
        };
        drop(recs);
        if std::env::var_os("DISKLENS_PROFILE").is_some() {
            eprintln!(
                "[profile] mft: {total_records} records ({} MiB) in {extent_count} extents, {readers} readers/{parsers} parsers: read {} ms, read+parse {parse_ms} ms, total {} ms; parsed records {records_mib} MiB; {} orphans, {} extra hard-link names",
                (total_records * rec_size) >> 20,
                read_done_ms.load(Ordering::Relaxed),
                start.elapsed().as_millis(),
                stats.orphans,
                stats.hardlink_names
            );
        }
        Ok(())
    }
}