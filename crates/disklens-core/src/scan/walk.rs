//! Parallel directory walk.
//!
//! Worker threads pull folder tasks from a shared queue and enumerate them. One builder thread (the
//! caller) owns all tree mutation: it appends each folder's entries as a contiguous child range, dedups
//! hard links by file id, rolls totals up the ancestor chain, and enqueues the sub-folders. Because only
//! the builder enqueues, "tasks sent == results received" means the walk is finished.

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::thread;

use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};

use super::enumerate::{self, Level, Strategy};
use super::{root_display_name, Inaccessible, Progress};
use crate::tree::{flags, NodeId, RawEntry, Tree, ROOT};

struct Task {
    id: NodeId,
    path: String,
}

enum Msg {
    Batch { id: NodeId, path: String, entries: Vec<RawEntry> },
    Failed { id: NodeId, path: String, error: std::io::Error },
    Skipped,
}

/// Minimal Fx-style hasher: file ids are already well distributed.
#[derive(Default)]
struct IdHasher(u64);
impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("only u64 keys")
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}
type IdSet = HashSet<u64, BuildHasherDefault<IdHasher>>;

fn fold_id(id: u128) -> u64 {
    (id as u64) ^ ((id >> 64) as u64).rotate_left(29)
}

fn join(parent: &str, name: &str) -> String {
    let mut s = String::with_capacity(parent.len() + name.len() + 1);
    s.push_str(parent);
    if !parent.ends_with('\\') {
        s.push('\\');
    }
    s.push_str(name);
    s
}

pub(crate) fn parallel_walk(root: &Path, strategy: Strategy, threads: usize, progress: &Progress, tree: &Arc<RwLock<Tree>>) -> Vec<Inaccessible> {
    let root_path = root_display_name(root);
    let cluster = enumerate::cluster_size(&root_path[..root_path.len().min(3)]);
    let level = Arc::new(Level::new(strategy));
    let (task_tx, task_rx) = unbounded::<Task>();
    let (msg_tx, msg_rx) = unbounded::<Msg>();
    let profile = std::env::var_os("DISKLENS_PROFILE").is_some();
    let enum_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let rx: Receiver<Task> = task_rx.clone();
            let tx: Sender<Msg> = msg_tx.clone();
            let level = level.clone();
            let enum_ns = enum_ns.clone();
            let cancel = &progress.cancel as *const std::sync::atomic::AtomicBool as usize;
            thread::spawn(move || {
                // SAFETY: `progress` outlives the walk; all workers are joined before returning.
                let cancel = unsafe { &*(cancel as *const std::sync::atomic::AtomicBool) };
                let mut buf: Vec<u64> = Vec::new();
                for task in rx.iter() {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = tx.send(Msg::Skipped);
                        continue;
                    }
                    let mut entries = Vec::new();
                    let t0 = std::time::Instant::now();
                    let r = enumerate::enumerate_dir(&task.path, &level, cluster, &mut buf, &mut entries);
                    enum_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let msg = match r {
                        Ok(()) => Msg::Batch { id: task.id, path: task.path, entries },
                        Err(error) => Msg::Failed { id: task.id, path: task.path, error },
                    };
                    let _ = tx.send(msg);
                }
            })
        })
        .collect();
    drop(task_rx);
    drop(msg_tx);

    let mut pending = 1usize;
    task_tx.send(Task { id: ROOT, path: root_path.clone() }).unwrap();
    let mut seen = IdSet::default();
    let mut inaccessible = Vec::new();
    let mut subdirs: Vec<Task> = Vec::new();
    let (mut busy, mut idle) = (std::time::Duration::ZERO, std::time::Duration::ZERO);

    'outer: while pending > 0 {
        // Block for one message, then drain whatever else is ready under a single write lock.
        let w0 = std::time::Instant::now();
        let Ok(first) = msg_rx.recv() else { break };
        let b0 = std::time::Instant::now();
        idle += b0 - w0;
        let mut t = tree.write().unwrap();
        let mut next = Some(first);
        let mut drained = 0;
        while let Some(msg) = next.take() {
            pending -= 1;
            drained += 1;
            match msg {
                Msg::Batch { id, path: parent_path, mut entries } => {
                    let (mut files, mut bytes, mut dirs) = (0u64, 0u64, 0u64);
                    for e in entries.iter_mut() {
                        if e.is_dir {
                            dirs += 1;
                            continue;
                        }
                        if e.flags & flags::LINK != 0 {
                            continue;
                        }
                        if let Some(fid) = e.file_id {
                            if !seen.insert(fold_id(fid)) {
                                e.flags |= flags::HARDLINK_DUP;
                                continue;
                            }
                        }
                        files += 1;
                        bytes += e.size;
                    }
                    let first_child = t.add_children(id, &entries);
                    progress.files.fetch_add(files, Ordering::Relaxed);
                    progress.dirs.fetch_add(dirs, Ordering::Relaxed);
                    progress.bytes.fetch_add(bytes, Ordering::Relaxed);
                    if !progress.is_cancelled() {
                        for (k, e) in entries.iter().enumerate() {
                            if e.is_dir && e.flags & flags::LINK == 0 {
                                let cid = first_child + k as NodeId;
                                subdirs.push(Task { id: cid, path: join(&parent_path, &e.name) });
                            }
                        }
                    }
                }
                Msg::Failed { id, path, error } => {
                    t.mark(id, flags::INACCESSIBLE | flags::SCANNED);
                    progress.errors.fetch_add(1, Ordering::Relaxed);
                    inaccessible.push(Inaccessible { path, error: error.to_string() });
                }
                Msg::Skipped => {}
            }
            if drained < 256 {
                next = match msg_rx.try_recv() {
                    Ok(m) => Some(m),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => break 'outer,
                };
            }
        }
        drop(t);
        for task in subdirs.drain(..) {
            pending += 1;
            task_tx.send(task).unwrap();
        }
        busy += b0.elapsed();
    }
    if profile {
        eprintln!("[profile] threads {threads}: enumerate {:.1}s total across workers, builder busy {:.2}s idle {:.2}s", enum_ns.load(Ordering::Relaxed) as f64 / 1e9, busy.as_secs_f64(), idle.as_secs_f64());
    }
    drop(task_tx);
    for w in workers {
        let _ = w.join();
    }
    tree.write().unwrap().shrink_to_fit();
    inaccessible
}
