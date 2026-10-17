//! Treemap layout properties.

use havespace_core::layout::{hit_test, layout, squarify, Block, LayoutOptions, Rect};
use havespace_core::tree::{RawEntry, Tree, ROOT};

const EPS: f32 = 0.05;

fn file(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size, mtime: 0, flags: 0, file_id: None }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x + EPS < b.x + b.w && b.x + EPS < a.x + a.w && a.y + EPS < b.y + b.h && b.y + EPS < a.y + a.h
}

#[test]
fn squarify_areas_are_proportional_and_fill_the_rect() {
    let sizes = [500.0, 250.0, 120.0, 80.0, 30.0, 15.0, 5.0];
    let r = Rect { x: 10.0, y: 20.0, w: 640.0, h: 360.0 };
    let rects = squarify(&sizes, r);
    let total: f64 = sizes.iter().sum();
    let mut sum_area = 0.0;
    for (s, q) in sizes.iter().zip(&rects) {
        let expected = (*s / total) as f32 * r.area();
        assert!((q.area() - expected).abs() / expected < 0.001, "area {} vs {}", q.area(), expected);
        assert!(q.x >= r.x - EPS && q.y >= r.y - EPS && q.x + q.w <= r.x + r.w + EPS && q.y + q.h <= r.y + r.h + EPS);
        sum_area += q.area();
    }
    assert!((sum_area - r.area()).abs() < 1.0);
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            assert!(!overlaps(&rects[i], &rects[j]), "{i} overlaps {j}");
        }
    }
}

#[test]
fn squarify_keeps_aspect_ratios_reasonable() {
    let sizes: Vec<f64> = (1..=40).rev().map(|v| v as f64).collect();
    let rects = squarify(&sizes, Rect { x: 0.0, y: 0.0, w: 1000.0, h: 600.0 });
    let worst = rects.iter().map(|r| (r.w / r.h).max(r.h / r.w)).fold(0.0f32, f32::max);
    assert!(worst < 4.0, "worst aspect {worst}");
}

#[test]
fn squarify_handles_edge_cases() {
    assert!(squarify(&[], Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }).is_empty());
    let one = squarify(&[42.0], Rect { x: 0.0, y: 0.0, w: 10.0, h: 5.0 });
    assert_eq!(one[0], Rect { x: 0.0, y: 0.0, w: 10.0, h: 5.0 });
    let zero = squarify(&[1.0, 1.0], Rect { x: 0.0, y: 0.0, w: 0.0, h: 5.0 });
    assert!(zero.iter().all(|r| r.area() == 0.0));
}

/// Root with a couple of nested folders and many tiny files to trigger merging.
fn tree() -> Tree {
    let mut t = Tree::new("C:\\");
    let f = t.add_children(ROOT, &[dir("big"), dir("medium"), file("huge.iso", 4_000_000_000), dir("tiny-files")]);
    let big = t.add_children(f, &[file("a.mkv", 2_000_000_000), file("b.mkv", 1_000_000_000), dir("inner")]);
    t.add_children(big + 2, &[file("c.mp4", 600_000_000), file("d.mp4", 400_000_000)]);
    t.add_children(f + 1, &[file("e.zip", 900_000_000), file("f.zip", 100_000_000)]);
    let tiny: Vec<RawEntry> = (0..5000).map(|i| file(&format!("t{i}.txt"), 10_000)).collect();
    t.add_children(f + 3, &tiny);
    t
}

fn contains(outer: &Block, inner: &Block) -> bool {
    inner.x >= outer.x - EPS && inner.y >= outer.y - EPS && inner.x + inner.w <= outer.x + outer.w + EPS && inner.y + inner.h <= outer.y + outer.h + EPS
}

#[test]
fn nested_layout_children_stay_inside_parents_and_siblings_do_not_overlap() {
    let t = tree();
    let blocks = layout(&t, ROOT, 1200.0, 800.0, &LayoutOptions::default());
    assert!(!blocks.is_empty());
    for (i, b) in blocks.iter().enumerate() {
        assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.w <= 1200.0 + EPS && b.y + b.h <= 800.0 + EPS);
        if b.parent >= 0 {
            let p = &blocks[b.parent as usize];
            assert!(contains(p, b), "block {i} escapes its parent");
            assert!(b.y >= p.y + p.header - EPS, "block {i} covers its parent's header");
            assert!((b.parent as usize) < i, "parents are drawn before children");
        }
    }
    for i in 0..blocks.len() {
        for j in i + 1..blocks.len() {
            if blocks[i].parent == blocks[j].parent {
                let (a, b) = (&blocks[i], &blocks[j]);
                let ra = Rect { x: a.x, y: a.y, w: a.w, h: a.h };
                let rb = Rect { x: b.x, y: b.y, w: b.w, h: b.h };
                assert!(!overlaps(&ra, &rb), "siblings {i} and {j} overlap");
            }
        }
    }
}

#[test]
fn top_level_areas_track_sizes() {
    let t = tree();
    let o = LayoutOptions { gap: 0.0, ..Default::default() };
    let blocks = layout(&t, ROOT, 1000.0, 1000.0, &o);
    let top: Vec<&Block> = blocks.iter().filter(|b| b.parent < 0).collect();
    let total = t.size[0] as f64;
    for b in top {
        let expected = b.size as f64 / total * 1_000_000.0;
        let got = (b.w * b.h) as f64;
        assert!((got - expected).abs() / expected < 0.01, "{} got {got} expected {expected}", t.name(b.node));
    }
}

#[test]
fn tiny_children_merge_into_one_aggregate() {
    let t = tree();
    let blocks = layout(&t, ROOT, 1200.0, 800.0, &LayoutOptions::default());
    let tiny_dir = t.child_by_name(ROOT, "tiny-files").unwrap();
    let inside: Vec<&Block> = blocks.iter().filter(|b| b.parent >= 0 && blocks[b.parent as usize].node == tiny_dir).collect();
    // 5000 files of 10 KB in a small folder can't all be drawn.
    assert!(inside.len() < 5000);
    let agg: Vec<_> = inside.iter().filter(|b| b.aggregate).collect();
    assert!(agg.len() <= 1);
    let drawn: u64 = inside.iter().map(|b| b.size).sum();
    assert!(drawn <= t.size[tiny_dir as usize]);
}

#[test]
fn max_blocks_is_respected_and_hit_test_finds_deepest() {
    let t = tree();
    let o = LayoutOptions { max_blocks: 5, ..Default::default() };
    assert!(layout(&t, ROOT, 1200.0, 800.0, &o).len() <= 5);

    let blocks = layout(&t, ROOT, 1200.0, 800.0, &LayoutOptions::default());
    let deepest = blocks.iter().enumerate().max_by_key(|(_, b)| b.depth).unwrap();
    let (cx, cy) = (deepest.1.x + deepest.1.w / 2.0, deepest.1.y + deepest.1.h / 2.0);
    assert_eq!(hit_test(&blocks, cx, cy), Some(deepest.0));
    assert_eq!(hit_test(&blocks, -5.0, -5.0), None);
}

fn flat_tree(dirs: u32, files_per_dir: u32) -> Tree {
    let mut t = Tree::new("C:\\");
    let ds: Vec<RawEntry> = (0..dirs).map(|i| dir(&format!("d{i}"))).collect();
    let f = t.add_children(ROOT, &ds);
    for d in 0..dirs {
        let files: Vec<RawEntry> = (0..files_per_dir).map(|i| file(&format!("f{i}"), 50_000 + (i as u64 * 7919 % 1000))).collect();
        t.add_children(f + d, &files);
    }
    t
}

#[test]
fn layout_of_900k_files_is_fast() {
    // 1600×900 px can't show 900k files (1.6 px² each), so most merge; the point is speed.
    let t = flat_tree(30, 30_000);
    let start = std::time::Instant::now();
    let blocks = layout(&t, ROOT, 1600.0, 900.0, &LayoutOptions { min_area: 4.0, ..Default::default() });
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("layout of 900k files -> {} blocks in {ms:.1} ms", blocks.len());
    assert!(ms < 2000.0);
}

#[test]
fn layout_draws_tens_of_thousands_of_visible_blocks() {
    // 60k files at ~24 px² each: all should be drawn individually.
    let t = flat_tree(30, 2000);
    let start = std::time::Instant::now();
    let blocks = layout(&t, ROOT, 1600.0, 900.0, &LayoutOptions { min_area: 4.0, ..Default::default() });
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("layout of 60k files -> {} blocks in {ms:.1} ms", blocks.len());
    assert!(blocks.len() > 50_000, "{} blocks", blocks.len());
}
