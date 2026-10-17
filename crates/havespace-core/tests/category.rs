use havespace_core::category::{category_totals, dominant_categories, ext_categories, Category};
use havespace_core::tree::{RawEntry, Tree, ROOT};

fn file(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size, mtime: 0, flags: 0, file_id: None }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}

#[test]
fn dominant_category_follows_bytes_not_counts() {
    let mut t = Tree::new("C:\\");
    let f = t.add_children(ROOT, &[dir("media"), dir("empty"), file("x.zip", 10)]);
    let m = t.add_children(f, &[file("a.mp4", 5000), file("b.jpg", 10), file("c.jpg", 10), file("d.jpg", 10), dir("sub")]);
    t.add_children(m + 4, &[file("e.png", 4000)]);
    t.add_children(f + 1, &[]);
    let ec = ext_categories(&t);
    let dom = dominant_categories(&t, &ec);
    assert_eq!(dom[f as usize], Category::Video as u8);
    assert_eq!(dom[(m + 4) as usize], Category::Image as u8);
    assert_eq!(dom[(f + 1) as usize], Category::Folder as u8);
    assert_eq!(dom[0], Category::Video as u8);

    let totals = category_totals(&t, &ec);
    assert_eq!(totals[0].category, Category::Video);
    assert_eq!(totals[1].category, Category::Image);
    assert_eq!(totals[1].size, 4030);
    assert_eq!(totals[1].files, 4);
}

#[test]
fn swap_files_are_system_not_executables() {
    use havespace_core::category::category_of;
    let mut t = Tree::new("C:\\");
    let f = t.add_children(ROOT, &[file("pagefile.sys", 100), file("HIBERFIL.SYS", 50), file("ntfs.sys", 5)]);
    let ec = ext_categories(&t);
    assert_eq!(category_of(&t, &ec, f), Category::System);
    assert_eq!(category_of(&t, &ec, f + 1), Category::System);
    assert_eq!(category_of(&t, &ec, f + 2), Category::Executable);
}

#[test]
fn extension_mapping() {
    assert_eq!(Category::from_ext("mkv"), Category::Video);
    assert_eq!(Category::from_ext("vhdx"), Category::System);
    assert_eq!(Category::from_ext("dll"), Category::Executable);
    assert_eq!(Category::from_ext("unknownext"), Category::Other);
}
