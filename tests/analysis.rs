//! Known-answer tests for the classifier.
//!
//! Every figure here is a published property of the pattern, so these check
//! the analyser and the engine underneath it at the same time.

use cgol_bestiary::analysis::{analyse, Budget, Class};
use cgol_bestiary::pattern::{self, Pattern};

fn classify(rle: &str, budget: Budget) -> cgol_bestiary::Analysis {
    analyse(&Pattern::parse_rle(rle).expect("valid RLE"), budget)
}

fn small() -> Budget {
    Budget { max_generations: 400, margin: 120, ..Budget::default() }
}

#[test]
fn block_is_a_still_life() {
    let a = classify("x = 2, y = 2, rule = B3/S23\n2o$2o!", small());
    assert_eq!(a.class, Class::StillLife);
    assert_eq!(a.period(), Some(1));
    assert_eq!(a.category(), "still-life");
}

#[test]
fn blinker_is_a_period_two_oscillator() {
    let a = classify(pattern::BLINKER, small());
    assert_eq!(a.class, Class::Oscillator { period: 2 });
    assert_eq!(a.category(), "oscillator");
}

#[test]
fn pulsar_is_a_period_three_oscillator() {
    let pulsar = "x = 13, y = 13, rule = B3/S23\n\
        2b3o3b3o2b2$o4bobo4bo$o4bobo4bo$o4bobo4bo$2b3o3b3o2b2$\
        2b3o3b3o2b$o4bobo4bo$o4bobo4bo$o4bobo4bo2$2b3o3b3o!";
    let a = classify(pulsar, small());
    assert_eq!(a.class, Class::Oscillator { period: 3 });
}

#[test]
fn glider_is_c_over_4_diagonal() {
    let a = classify(pattern::GLIDER, small());
    assert!(matches!(a.class, Class::Spaceship { period: 4, .. }), "got {:?}", a.class);
    assert_eq!(a.speed().as_deref(), Some("c/4 diagonal"));
    assert_eq!(a.category(), "spaceship");
}

#[test]
fn lightweight_spaceship_is_c_over_2_orthogonal() {
    let lwss = "x = 5, y = 4, rule = B3/S23\nbo2bo$o4b$o3bo$4o!";
    let a = classify(lwss, small());
    assert!(matches!(a.class, Class::Spaceship { period: 4, .. }), "got {:?}", a.class);
    assert_eq!(a.speed().as_deref(), Some("c/2 orthogonal"));
}

#[test]
fn gosper_gun_grows_without_bound() {
    let a = classify(pattern::GOSPER_GLIDER_GUN, small());
    assert_eq!(a.class, Class::InfiniteGrowth);
    assert_eq!(a.category(), "infinite-growth");
}

#[test]
fn r_pentomino_stabilises_at_1103() {
    // The population, not the configuration — six gliders escape and keep
    // going forever, so the configuration never repeats.
    let a = classify(pattern::R_PENTOMINO, Budget { max_generations: 1600, margin: 480, ..Budget::default() });
    let Class::Stabilises { at, .. } = a.class else {
        panic!("expected the R-pentomino to stabilise, got {:?}", a.class);
    };
    assert_eq!(at, 1103);
    assert_eq!(a.category(), "methuselah");
}

#[test]
fn acorn_stabilises_at_5206() {
    let a = classify(pattern::ACORN, Budget { max_generations: 6000, margin: 1500, max_grid: 4096, ..Budget::default() });
    let Class::Stabilises { at, .. } = a.class else {
        panic!("expected Acorn to stabilise, got {:?}", a.class);
    };
    assert_eq!(at, 5206);
    assert_eq!(a.category(), "methuselah");
}

#[test]
fn a_lone_cell_dies_immediately() {
    let a = classify("x = 1, y = 1, rule = B3/S23\no!", small());
    assert_eq!(a.class, Class::Dies { at: 1 });
}

#[test]
fn oversized_patterns_are_reported_not_simulated() {
    let p = Pattern {
        width: 100_000,
        height: 4,
        live: vec![(0, 0), (99_999, 3)],
        ..Pattern::default()
    };
    let a = analyse(&p, Budget { max_grid: 2048, ..Budget::default() });
    assert_eq!(a.class, Class::TooLarge);
    assert_eq!(a.generations, 0);
}

#[test]
fn speed_notation_reduces_fractions() {
    use cgol_bestiary::analysis::speed_string;
    assert_eq!(speed_string(1, 1, 4), "c/4 diagonal");
    assert_eq!(speed_string(0, 2, 4), "c/2 orthogonal");
    assert_eq!(speed_string(4, 0, 8), "c/2 orthogonal");
    assert_eq!(speed_string(2, 0, 5), "2c/5 orthogonal");
    assert_eq!(speed_string(1, 2, 6), "(1,2)c/6 oblique");
}

#[test]
fn rejects_patterns_in_other_rules() {
    let highlife = Pattern::parse_rle("x = 3, y = 3, rule = B36/S23\nbob$2bo$3o!").unwrap();
    assert!(!highlife.is_life());

    // Both spellings of Conway's rule, plus the old survival-first notation.
    for r in ["B3/S23", "b3/s23", "23/3", "S23/B3", "B3/S23:T64,64"] {
        let src = format!("x = 3, y = 1, rule = {r}\n3o!");
        assert!(Pattern::parse_rle(&src).unwrap().is_life(), "{r} should be Life");
    }
    // No rule at all means Life by convention.
    assert!(Pattern::parse_rle("x = 3, y = 1\n3o!").unwrap().is_life());
}
