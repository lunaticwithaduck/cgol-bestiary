//! The HashLife backend, checked against the engine we already trust.
//!
//! `golback` is a young crate with almost no users, so the point of these
//! tests is not to confirm it compiles — it is to hold it against the
//! bit-parallel engine, which is itself pinned to published Life results.

use conway::bitgrid::Rng;
use conway::hashlife::{HashWorld, MAX_CELLS};
use conway::{BitGrid, Boundary, Macrocell};
use std::collections::BTreeSet;

/// Cells normalised so the top-left of the bounding box sits at the origin,
/// which makes two engines comparable regardless of where each put the pattern.
fn normalised(cells: &[(i64, i64)]) -> BTreeSet<(i64, i64)> {
    let x0 = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let y0 = cells.iter().map(|c| c.1).min().unwrap_or(0);
    cells.iter().map(|&(x, y)| (x - x0, y - y0)).collect()
}

fn bitmap_cells(g: &BitGrid) -> Vec<(i64, i64)> {
    let mut v = Vec::new();
    g.live_cells_into(&mut v);
    v.into_iter().map(|(x, y)| (x as i64, y as i64)).collect()
}

const GLIDER: [(i64, i64); 5] = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

#[test]
fn glider_moves_one_cell_diagonally_every_four_generations() {
    let mut hw = HashWorld::new();
    assert!(hw.load_cells(&GLIDER));
    assert_eq!(hw.population(), 5.0);

    let (x0, y0, ..) = hw.bbox();
    let shape = normalised(hw.snapshot_cells());
    hw.step(4);

    assert_eq!(hw.generation(), 4.0);
    assert_eq!(hw.population(), 5.0);
    assert_eq!(normalised(hw.snapshot_cells()), shape, "a glider keeps its shape");
    let (x1, y1, ..) = hw.bbox();
    assert_eq!(((x1 - x0).abs(), (y1 - y0).abs()), (1, 1));
}

/// The important one: same soup, two independently written engines, compared
/// cell for cell every generation.
#[test]
fn agrees_with_the_bitmap_engine_on_a_soup() {
    for (seed, density) in [(1u64, 30u32), (2, 45), (3, 60)] {
        let mut rng = Rng::new(seed);
        let mut seeded = Vec::new();
        for y in 0..24i64 {
            for x in 0..24i64 {
                if rng.next() % 100 < density as u64 {
                    seeded.push((x, y));
                }
            }
        }

        let mut hw = HashWorld::new();
        assert!(hw.load_cells(&seeded));

        // A dead boundary with plenty of margin behaves like the infinite
        // plane for as long as nothing gets near the edge.
        let mut bm = BitGrid::with_boundary(512, 512, Boundary::Dead);
        for &(x, y) in &seeded {
            bm.set(x as usize + 240, y as usize + 240, true);
        }

        for gen in 1..=80 {
            hw.step(1);
            bm.step();
            let ctx = format!("seed={seed} density={density} gen={gen}");
            assert_eq!(
                hw.population() as u64,
                bm.population(),
                "{ctx}: populations diverge"
            );
            assert_eq!(
                normalised(hw.snapshot_cells()),
                normalised(&bitmap_cells(&bm)),
                "{ctx}: cell layouts diverge"
            );
        }
    }
}

#[test]
fn big_steps_land_in_the_same_place_as_small_ones() {
    // HashLife's whole point is jumping many generations at once; it had
    // better agree with itself about where that lands.
    let mut a = HashWorld::new();
    let mut b = HashWorld::new();
    a.load_cells(&GLIDER);
    b.load_cells(&GLIDER);

    a.step(512);
    for _ in 0..512 {
        b.step(1);
    }
    assert_eq!(a.generation(), b.generation());
    assert_eq!(a.bbox(), b.bbox(), "one jump of 512 vs 512 single steps");
    assert_eq!(normalised(a.snapshot_cells()), normalised(b.snapshot_cells()));
}

/// The universe grows itself to follow a travelling pattern.
///
/// Before `ensure_room_for`, a glider in a universe sized to its own bounding
/// box decayed into a 2x2 block after 128 cells of travel, with no error — the
/// nastiest failure mode this code had.
#[test]
fn the_universe_grows_to_follow_a_travelling_pattern() {
    let mut hw = HashWorld::new();
    hw.load_cells(&GLIDER);
    let level0 = hw.level();
    let (x0, y0, ..) = hw.bbox();

    // A glider covers one cell every four generations, so this carries it two
    // million cells out of a universe that started a few dozen across.
    hw.step(8_000_000);

    assert!(!hw.clipped(), "nowhere near the i64 growth limit");
    assert_eq!(hw.population(), 5.0, "still an intact glider, not wall debris");
    assert!(hw.level() > level0, "universe should have grown past 2^{level0}");

    let (x1, y1, ..) = hw.bbox();
    assert_eq!((x1 - x0).abs(), 2_000_000, "displacement along x");
    assert_eq!((y1 - y0).abs(), 2_000_000, "displacement along y");
}

#[test]
fn quadtree_rendering_matches_plotting_every_cell() {
    let mut hw = HashWorld::new();
    hw.load_cells(&GLIDER);
    hw.step(37); // an arbitrary phase, not a tidy multiple of anything

    for &(cam_x, cam_y) in &[(0.0, 0.0), (-13.0, 7.0), (5.5, -2.5), (-1000.0, -1000.0)] {
        for &scale in &[1i32, 2, 3, 8, -1, -2, -4, -8, -16] {
            for &(w, h) in &[(64usize, 48usize), (37, 19), (128, 128)] {
                hw.render(cam_x, cam_y, scale, w, h);
                let walked = hw.rgba().to_vec();
                let plotted = hw.render_from_cells(cam_x, cam_y, scale, w, h).to_vec();
                assert_eq!(
                    walked.len(),
                    plotted.len(),
                    "buffer size at cam=({cam_x},{cam_y}) scale={scale} {w}x{h}"
                );
                let diff = walked.iter().zip(&plotted).filter(|(a, b)| a != b).count();
                assert_eq!(
                    diff, 0,
                    "{diff} bytes differ at cam=({cam_x},{cam_y}) scale={scale} {w}x{h}"
                );
            }
        }
    }
}

/// Same check against a pattern with a hundred thousand cells spread over a
/// quarter-million-cell span, where the walk prunes almost the entire tree.
#[test]
fn quadtree_rendering_matches_on_a_real_pattern() {
    let Some(m) = load_file("demonoid-c512-hashlife-friendly.mc") else {
        eprintln!("skipping: not fetched");
        return;
    };
    let mut hw = HashWorld::new();
    hw.load_cells(&m.live_cells().unwrap());
    let (x0, y0, ..) = hw.bbox();

    for &scale in &[-1i32, -4, -256, -1024, -4096, 1, 4] {
        for &(cam_x, cam_y) in &[(x0 as f64, y0 as f64), (x0 as f64 - 5000.0, y0 as f64 + 1234.0)] {
            hw.render(cam_x, cam_y, scale, 200, 150);
            let walked = hw.rgba().to_vec();
            let plotted = hw.render_from_cells(cam_x, cam_y, scale, 200, 150).to_vec();
            let diff = walked.iter().zip(&plotted).filter(|(a, b)| a != b).count();
            assert_eq!(diff, 0, "{diff} bytes differ at scale={scale} cam=({cam_x},{cam_y})");
        }
    }
}

#[test]
fn an_empty_cell_list_is_refused() {
    let mut hw = HashWorld::new();
    assert!(!hw.load_cells(&[]));
}

#[test]
fn renders_into_an_rgba_viewport() {
    let mut hw = HashWorld::new();
    hw.load_cells(&GLIDER);
    let ptr = hw.render(-2.0, -2.0, 4, 64, 48);
    assert_eq!(hw.render_len(), 64 * 48 * 4);
    let px = unsafe { std::slice::from_raw_parts(ptr, hw.render_len() as usize) };
    // Five cells at four pixels square.
    let lit = px.chunks_exact(4).filter(|c| c[0] == 0x7e).count();
    assert_eq!(lit, 5 * 16, "got {lit} lit pixels");
}

// ------------------------------------------------- the real corpus ----
// These only run once ./fetch-patterns.sh has been used.

fn load_file(name: &str) -> Option<Macrocell> {
    let raw = std::fs::read(format!("www/patterns-mc/{name}")).ok()?;
    Macrocell::parse(&String::from_utf8_lossy(&raw)).ok()
}

#[test]
fn macrocell_population_survives_the_round_trip() {
    // Our parser counts population structurally from the DAG. golback rebuilds
    // the tree from a flat cell list. They arrive at it by entirely different
    // routes, so agreement is a real check on both.
    for name in [
        "demonoid-c512-hashlife-friendly.mc",
        "linear-propagator-p237228340.mc",
        "loafer-gun-p8388608-linear.mc",
        "logarithmic-width.mc",
        "ruler.mc",
    ] {
        let Some(m) = load_file(name) else {
            eprintln!("skipping {name}: not fetched");
            continue;
        };
        let cells = m.live_cells().expect("bbox fits i64");
        assert_eq!(cells.len() as u128, m.population, "{name}: enumeration count");

        let mut hw = HashWorld::new();
        assert!(hw.load_cells(&cells), "{name}: should load");
        assert_eq!(hw.population(), m.population as f64, "{name}: after rebuild");
    }
}

#[test]
fn oversized_patterns_are_refused_rather_than_attempted() {
    let Some(m) = load_file("metapixel-parity64.mc") else {
        eprintln!("skipping: not fetched");
        return;
    };
    assert!(m.population > MAX_CELLS, "expected this one to be over the ceiling");

    let mut hw = HashWorld::new();
    // Reject before enumerating: 100 million pairs would be 1.6GB.
    let src = std::fs::read("www/patterns-mc/metapixel-parity64.mc").unwrap();
    let code = hw.load_macrocell(&String::from_utf8_lossy(&src)) as u32;
    assert_eq!(code, 3, "should report TooManyCells");
}

/// The payoff: a pattern that builds a complete copy of itself.
///
/// The file documents a step of 4096 cells diagonally with a period of
/// 2^21 = 2,097,152 ticks. Takes about ten seconds.
#[test]
fn the_demonoid_replicates_itself() {
    let Some(m) = load_file("demonoid-c512-hashlife-friendly.mc") else {
        eprintln!("skipping: not fetched");
        return;
    };
    let mut hw = HashWorld::new();
    assert!(hw.load_cells(&m.live_cells().unwrap()));

    let (x0, y0, ..) = hw.bbox();
    let pop = hw.population();
    hw.step(2_097_152);

    let (x1, y1, ..) = hw.bbox();
    assert_eq!((x1 - x0).abs(), 4096, "step size along x");
    assert_eq!((y1 - y0).abs(), 4096, "step size along y");
    assert_eq!(hw.population(), pop, "a spaceship returns to its own population");
}
