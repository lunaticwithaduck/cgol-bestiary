//! Macrocell parsing, with hand-built files whose answers can be worked out
//! on paper.
//!
//! The risky part is the bounding-box recursion: child offsets double with
//! every level, so a wrong shift produces a number that still looks perfectly
//! reasonable. Hence the explicit quadrant tests.

use cgol_bestiary::macrocell::Macrocell;

/// An 8x8 leaf holding a glider at its top-left corner. Five cells, spanning
/// (0,0)..(2,2).
const GLIDER_LEAF: &str = ".*$..*$***$";

fn parse(body: &str) -> Macrocell {
    Macrocell::parse(body).expect("should parse")
}

#[test]
fn reads_a_single_leaf() {
    let m = parse(&format!("[M2] (golly 2.0)\n#R B3/S23\n{GLIDER_LEAF}\n"));
    assert!(m.is_life());
    assert_eq!(m.population, 5);
    assert_eq!(m.level, 3, "a bare leaf is a level-3 node");
    let b = m.bbox.expect("has live cells");
    assert_eq!((b.min_x, b.min_y, b.max_x, b.max_y), (0, 0, 2, 2));
    assert_eq!((b.width(), b.height()), (3, 3));
}

#[test]
fn places_a_leaf_in_the_north_west_quadrant() {
    let m = parse(&format!("[M2]\n{GLIDER_LEAF}\n4 1 0 0 0\n"));
    assert_eq!(m.level, 4);
    assert_eq!(m.population, 5);
    let b = m.bbox.unwrap();
    assert_eq!((b.min_x, b.min_y, b.max_x, b.max_y), (0, 0, 2, 2));
}

#[test]
fn offsets_each_quadrant_by_half_the_node() {
    // A level-4 node is 16 wide, so its south-east child starts at (8, 8).
    for (line, expect) in [
        ("4 1 0 0 0", (0u128, 0u128)),
        ("4 0 1 0 0", (8, 0)),
        ("4 0 0 1 0", (0, 8)),
        ("4 0 0 0 1", (8, 8)),
    ] {
        let m = parse(&format!("[M2]\n{GLIDER_LEAF}\n{line}\n"));
        let b = m.bbox.unwrap();
        assert_eq!((b.min_x, b.min_y), expect, "for {line}");
        assert_eq!((b.width(), b.height()), (3, 3), "for {line}");
    }
}

#[test]
fn a_shared_node_is_counted_once_per_use() {
    // The same leaf in two quadrants. Memoising its population must not stop
    // it being added twice.
    let m = parse(&format!("[M2]\n{GLIDER_LEAF}\n4 1 0 0 1\n"));
    assert_eq!(m.population, 10);
    let b = m.bbox.unwrap();
    assert_eq!((b.min_x, b.min_y, b.max_x, b.max_y), (0, 0, 10, 10));
    assert_eq!(m.nodes, 2);
}

#[test]
fn offsets_compound_through_levels() {
    // level 5 is 32 wide, so its SE child sits at (16, 16); inside that
    // level-4 node the glider is at its own origin.
    let m = parse(&format!("[M2]\n{GLIDER_LEAF}\n4 1 0 0 0\n5 0 0 0 2\n"));
    assert_eq!(m.level, 5);
    assert_eq!(m.population, 5);
    let b = m.bbox.unwrap();
    assert_eq!((b.min_x, b.min_y, b.max_x, b.max_y), (16, 16, 18, 18));
}

#[test]
fn deep_sparse_trees_do_not_overflow() {
    // A chain of 60 levels: one glider in a universe 2^63 cells square.
    let mut src = format!("[M2]\n{GLIDER_LEAF}\n");
    for level in 4..=63 {
        // The leaf is node 1, so the branch added at each level names the one
        // defined just before it.
        src.push_str(&format!("{level} {} 0 0 0\n", level - 3));
    }
    let m = parse(&src);
    assert_eq!(m.level, 63);
    assert_eq!(m.population, 5);
    let b = m.bbox.unwrap();
    assert_eq!((b.min_x, b.min_y, b.max_x, b.max_y), (0, 0, 2, 2));
}

#[test]
fn an_empty_universe_has_no_bounding_box() {
    let m = parse("[M2]\n.$\n4 0 0 0 0\n");
    assert_eq!(m.population, 0);
    assert!(m.bbox.is_none());
}

#[test]
fn no_rule_line_means_life() {
    assert!(parse(&format!("[M2]\n{GLIDER_LEAF}\n")).is_life());
}

#[test]
fn rejects_other_rules() {
    for rule in [
        "B36/S23",                              // HighLife
        "B3-jknr4ity5ijk6i8/S23-a4city6c7c",    // the fangtian metacells
        "JvN29",
    ] {
        let m = parse(&format!("[M2]\n#R {rule}\n{GLIDER_LEAF}\n"));
        assert!(!m.is_life(), "{rule} should not be treated as Life");
    }
}

#[test]
fn captures_comments() {
    let m = parse("[M2]\n#C first\n#C second\n.*$\n");
    assert_eq!(m.comments, ["first", "second"]);
}

#[test]
fn rejects_malformed_input() {
    // Forward reference: a branch may only name nodes already defined.
    assert!(Macrocell::parse("[M2]\n.*$\n4 7 0 0 0\n").is_err());
    // Leaves are 8x8.
    assert!(Macrocell::parse("[M2]\n.$.$.$.$.$.$.$.$.*$\n").is_err());
    assert!(Macrocell::parse("[M2]\n.........*$\n").is_err());
    // Branch levels start at 4; below that it would be a leaf.
    assert!(Macrocell::parse("[M2]\n.*$\n3 1 0 0 0\n").is_err());
    assert!(Macrocell::parse("[M2]\n").is_err());
}

/// Only runs once ./fetch-patterns.sh has been used.
#[test]
fn reads_the_real_demonoid() {
    let path = "www/patterns-mc/demonoid-c512-hashlife-friendly.mc";
    let Ok(raw) = std::fs::read(path) else {
        eprintln!("skipping: {path} not fetched");
        return;
    };
    let m = Macrocell::parse(&String::from_utf8_lossy(&raw)).expect("real file should parse");
    assert!(m.is_life(), "the Demonoid is a Life pattern");
    assert!(m.population > 100_000, "population was {}", m.population);
    let b = m.bbox.expect("has cells");
    // Whatever the numbers are, the pattern has to fit inside its own universe.
    assert!(b.max_x < 1u128 << m.level && b.max_y < 1u128 << m.level);
}
