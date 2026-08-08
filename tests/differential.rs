//! The bit tricks in `bitgrid` are easy to get subtly wrong in ways that only
//! show up hundreds of generations in, or only at a word boundary. So: diff
//! every generation against the naive oracle.

use conway::bitgrid::Rng;
use conway::pattern::{self, Pattern};
use conway::{BitGrid, NaiveGrid};

fn assert_same(fast: &BitGrid, slow: &NaiveGrid, ctx: &str) {
    assert_eq!(fast.width(), slow.width(), "{ctx}: width");
    assert_eq!(fast.height(), slow.height(), "{ctx}: height");
    for y in 0..slow.height() {
        for x in 0..slow.width() {
            assert_eq!(
                fast.get(x, y),
                slow.get(x, y),
                "{ctx}: disagree at ({x}, {y}) on generation {}",
                fast.generation()
            );
        }
    }
}

/// Random soups, checked every single generation.
fn differential(width: usize, height: usize, seed: u64, density: u32, gens: usize) {
    let mut fast = BitGrid::new(width, height);
    let mut slow = NaiveGrid::new(width, height);

    let mut rng = Rng::new(seed);
    for y in 0..height {
        for x in 0..width {
            let alive = rng.next() % 100 < density as u64;
            fast.set(x, y, alive);
            slow.set(x, y, alive);
        }
    }

    let ctx = format!("{width}x{height} seed={seed} density={density}");
    assert_same(&fast, &slow, &ctx);
    for _ in 0..gens {
        fast.step();
        slow.step();
        assert_same(&fast, &slow, &ctx);
    }
}

#[test]
fn matches_oracle_single_word_rows() {
    // stride == 1, so a row's left and right neighbour words are the row
    // itself. Easiest case to get wrong, so it gets its own test.
    differential(64, 64, 1, 35, 120);
}

#[test]
fn matches_oracle_across_word_boundaries() {
    differential(128, 40, 2, 40, 120);
    differential(256, 17, 3, 30, 120);
}

#[test]
fn matches_oracle_sparse_and_dense() {
    differential(192, 24, 4, 8, 80);
    differential(192, 24, 5, 75, 80);
}

#[test]
fn matches_oracle_short_grids() {
    // Heights 3 and 4 are the tightest torus where a cell still has eight
    // distinct neighbours.
    differential(64, 3, 6, 45, 40);
    differential(128, 4, 7, 45, 40);
    differential(128, 5, 8, 45, 40);
}

#[test]
fn blinker_has_period_two() {
    let mut g = BitGrid::new(64, 64);
    let p = Pattern::parse_rle(pattern::BLINKER).unwrap();
    g.stamp(&p, 10, 10);

    let start: Vec<u64> = g.words().to_vec();
    g.step();
    assert_ne!(g.words(), start.as_slice(), "blinker should change");
    assert_eq!(g.population(), 3);
    g.step();
    assert_eq!(g.words(), start.as_slice(), "blinker should return after 2");
}

#[test]
fn block_is_still() {
    let mut g = BitGrid::new(64, 64);
    let p = Pattern::parse_rle("x = 2, y = 2, rule = B3/S23\n2o$2o!").unwrap();
    g.stamp(&p, 5, 5);
    let start: Vec<u64> = g.words().to_vec();
    g.step_n(10);
    assert_eq!(g.words(), start.as_slice(), "block should never move");
}

#[test]
fn glider_translates_diagonally() {
    let p = Pattern::parse_rle(pattern::GLIDER).unwrap();

    // Straddling a word boundary is the case worth checking: the glider has
    // to reassemble itself out of two different u64s.
    for start_x in [10, 61, 62, 63, 64] {
        let mut g = BitGrid::new(128, 64);
        g.stamp(&p, start_x, 20);
        g.step_n(4);

        let mut expected = BitGrid::new(128, 64);
        expected.stamp(&p, start_x + 1, 21);

        assert_eq!(
            g.words(),
            expected.words(),
            "glider from x={start_x} should land one cell down-right after 4 generations"
        );
    }
}

#[test]
fn glider_wraps_around_the_torus() {
    let p = Pattern::parse_rle(pattern::GLIDER).unwrap();
    let mut g = BitGrid::new(64, 64);
    g.stamp(&p, 0, 0);

    // 64 steps of 4 generations each brings it back to the start on a 64x64
    // torus, having crossed both seams.
    g.step_n(4 * 64);

    let mut expected = BitGrid::new(64, 64);
    expected.stamp(&p, 0, 0);
    assert_eq!(g.words(), expected.words(), "glider should return to origin");
}

#[test]
fn r_pentomino_settles_where_it_should() {
    // Famous result: the R-pentomino stabilises at generation 1103 with a
    // population of 116 on an unbounded board. Use a torus big enough that
    // nothing has wrapped by then.
    // Escaping gliders cover ~276 cells by then, so 1024 wide keeps everything
    // clear of the seams.
    let mut g = BitGrid::new(1024, 1024);
    let p = Pattern::parse_rle(pattern::R_PENTOMINO).unwrap();
    g.stamp(&p, 512, 512);
    g.step_n(1103);
    assert_eq!(g.population(), 116);
}

#[test]
fn gosper_gun_emits_a_glider_every_thirty_generations() {
    let mut g = BitGrid::new(256, 256);
    let p = Pattern::parse_rle(pattern::GOSPER_GLIDER_GUN).unwrap();
    g.stamp(&p, 10, 10);

    // The gun is 36 cells of period-30 machinery that adds 5 cells per cycle.
    let base = g.population();
    g.step_n(30);
    assert_eq!(g.population(), base + 5);
    g.step_n(30);
    assert_eq!(g.population(), base + 10);
}

#[test]
fn width_rounds_up_to_whole_words() {
    let g = BitGrid::new(100, 10);
    assert_eq!(g.width(), 128);
    assert_eq!(g.words().len(), 2 * 10);
}

#[test]
fn parses_rle() {
    let p = Pattern::parse_rle(pattern::GLIDER).unwrap();
    assert_eq!((p.width, p.height), (3, 3));
    let mut live = p.live.clone();
    live.sort();
    assert_eq!(live, [(0, 2), (1, 0), (1, 2), (2, 1), (2, 2)]);

    let gun = Pattern::parse_rle(pattern::GOSPER_GLIDER_GUN).unwrap();
    assert_eq!((gun.width, gun.height), (36, 9));
    assert_eq!(gun.live.len(), 36);
}

#[test]
fn rejects_garbage_rle() {
    assert!(Pattern::parse_rle("x = 3, y = 3, rule = B3/S23\nbo?b!").is_err());
    assert!(Pattern::parse_rle("x = oops, y = 3\nbo!").is_err());
}
