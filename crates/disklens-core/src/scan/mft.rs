//! NTFS Master File Table reader (milestone e). Until then it always declines, so `Method::Auto`
//! falls back to the directory walk.

use std::io;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::Progress;
use crate::tree::Tree;

pub fn scan_volume(_root: &Path, _progress: &Progress, _tree: &Arc<RwLock<Tree>>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "MFT reader not implemented yet"))
}
