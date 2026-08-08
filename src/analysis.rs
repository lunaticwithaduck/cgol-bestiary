//! Work out what a pattern *is* by running it.
//!
//! Every generation gets a translation-invariant fingerprint. If a
//! fingerprint recurs, the pattern has entered a cycle, and the displacement
//! between the two occurrences tells us whether it came back to the same place
//! (an oscillator) or somewhere else (a spaceship).
//!
//! Everything runs on a [`Boundary::Dead`] grid with a margin, and stops the
//! moment the pattern gets close enough to an edge that a cell could be born
//! off-grid — past that point we would no longer be simulating Life on the
//! infinite plane, and any verdict would be an artefact of the box.

use crate::bitgrid::{BitGrid, Boundary};
use crate::pattern::Pattern;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Nothing left alive.
    Dies { at: u32 },
    StillLife,
    Oscillator { period: u32 },
    Spaceship { period: u32, dx: i32, dy: i32 },
    /// Chaotic for a while, then falls into a cycle.
    Settles { at: u32, period: u32, dx: i32, dy: i32 },
    /// The configuration never repeats — escaping gliders keep travelling
    /// forever — but the population becomes periodic. This is the sense in
    /// which a methuselah is conventionally said to stabilise, and it is the
    /// only one that can catch Acorn or the R-pentomino.
    Stabilises { at: u32, period: u32 },
    /// Population climbs and never repeats — guns, puffers, rakes, breeders.
    InfiniteGrowth,
    /// Bigger than the analyser's grid cap; never simulated at all.
    TooLarge,
    /// Ran out of generations, space, or population budget with no verdict.
    Unresolved,
}

#[derive(Debug, Clone)]
pub struct Analysis {
    pub class: Class,
    pub initial_population: u32,
    pub final_population: u32,
    pub max_population: u32,
    /// How many generations were actually simulated.
    pub generations: u32,
    /// True if we stopped because the pattern reached the edge of the box.
    pub reached_edge: bool,
    pub grid: (usize, usize),
}

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_generations: u32,
    /// Blank space left around the pattern on every side.
    pub margin: usize,
    /// Hard cap on either grid dimension.
    pub max_grid: usize,
    /// Give up tracking once the pattern gets this big; it is a grower.
    pub max_population: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_generations: 3000, margin: 160, max_grid: 2048, max_population: 60_000 }
    }
}

impl Analysis {
    /// Stable slug for filtering in the UI.
    pub fn category(&self) -> &'static str {
        match self.class {
            Class::Dies { .. } => "dies",
            Class::StillLife => "still-life",
            Class::Oscillator { .. } => "oscillator",
            Class::Spaceship { .. } => "spaceship",
            // A methuselah is just a settler that took a long time from a
            // tiny start. The threshold is a convention, not a theorem.
            Class::Settles { at, .. } | Class::Stabilises { at, .. }
                if at >= 100 && self.initial_population <= 12 =>
            {
                "methuselah"
            }
            Class::Settles { .. } | Class::Stabilises { .. } => "settles",
            Class::InfiniteGrowth => "infinite-growth",
            Class::TooLarge => "too-large",
            Class::Unresolved => "unresolved",
        }
    }

    pub fn period(&self) -> Option<u32> {
        match self.class {
            Class::StillLife => Some(1),
            Class::Oscillator { period }
            | Class::Spaceship { period, .. }
            | Class::Settles { period, .. }
            | Class::Stabilises { period, .. } => Some(period),
            _ => None,
        }
    }

    /// Speed in Life's conventional notation, for spaceships only.
    pub fn speed(&self) -> Option<String> {
        match self.class {
            Class::Spaceship { period, dx, dy } => Some(speed_string(dx, dy, period)),
            _ => None,
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// `c` is one cell per generation, the speed of light in Life.
pub fn speed_string(dx: i32, dy: i32, period: u32) -> String {
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    let render = |num: u32, den: u32, kind: &str| {
        let g = gcd(num.max(1), den).max(1);
        let (n, d) = (num / g, den / g);
        if n == 1 {
            format!("c/{d} {kind}")
        } else {
            format!("{n}c/{d} {kind}")
        }
    };
    match (ax, ay) {
        (0, d) | (d, 0) => render(d, period, "orthogonal"),
        (a, b) if a == b => render(a, period, "diagonal"),
        (a, b) => format!("({a},{b})c/{period} oblique"),
    }
}

/// See [`crate::bitgrid::Signature::key`].
type SigKey = (u64, u32, u32, u32);

/// Longest period considered when looking for a periodic population tail.
const MAX_POP_PERIOD: usize = 64;

/// Find the earliest generation after which the population sequence is
/// periodic all the way to the end of the run.
///
/// Returns `(generation, period)`. The tail has to be long enough to not be a
/// coincidence — a constant population for three generations means nothing.
fn population_settles(pops: &[u32]) -> Option<(u32, u32)> {
    let n = pops.len();
    for p in 1..=MAX_POP_PERIOD.min(n / 8) {
        // Walk backwards for as long as the p-periodicity holds.
        let mut t = n - p;
        while t > 0 && pops[t - 1] == pops[t - 1 + p] {
            t -= 1;
        }
        let tail = n - t;
        if tail >= (8 * p).max(50) {
            return Some((t as u32, p as u32));
        }
    }
    None
}

/// Run `pattern` until it repeats, dies, escapes the box, or exhausts the
/// budget.
pub fn analyse(pattern: &Pattern, budget: Budget) -> Analysis {
    let w = (pattern.width + 2 * budget.margin).max(64);
    let h = (pattern.height + 2 * budget.margin).max(3);

    let too_big = pattern.width > budget.max_grid || pattern.height > budget.max_grid;
    let (w, h) = (w.min(budget.max_grid), h.min(budget.max_grid));

    let mut grid = BitGrid::with_boundary(w, h, Boundary::Dead);
    if too_big {
        return Analysis {
            class: Class::TooLarge,
            initial_population: pattern.live.len() as u32,
            final_population: pattern.live.len() as u32,
            max_population: pattern.live.len() as u32,
            generations: 0,
            reached_edge: true,
            grid: (grid.width(), grid.height()),
        };
    }
    grid.stamp_centred(pattern);

    let (gw, gh) = (grid.width() as u32, grid.height() as u32);
    // signature key -> (generation, min_x, min_y) of the first sighting
    let mut seen: HashMap<SigKey, (u32, u32, u32)> = HashMap::new();
    let mut buf = Vec::new();
    let mut pops: Vec<u32> = Vec::new();

    let initial_population = grid.population() as u32;
    let mut max_population = initial_population;
    let mut reached_edge = false;
    let mut class = Class::Unresolved;
    let mut generations = 0;

    for gen in 0..=budget.max_generations {
        let sig = grid.signature_into(&mut buf);
        generations = gen;
        pops.push(sig.population);
        max_population = max_population.max(sig.population);

        if sig.population == 0 {
            class = Class::Dies { at: gen };
            break;
        }
        if sig.population > budget.max_population {
            class = Class::InfiniteGrowth;
            break;
        }

        // One clear cell on every side, or a birth could happen off-grid on
        // the next step and we would silently lose it.
        if sig.min_x == 0 || sig.min_y == 0 || sig.max_x + 1 >= gw || sig.max_y + 1 >= gh {
            reached_edge = true;
            break;
        }

        match seen.entry(sig.key()) {
            std::collections::hash_map::Entry::Occupied(e) => {
                let &(g0, x0, y0) = e.get();
                let period = gen - g0;
                let dx = sig.min_x as i32 - x0 as i32;
                let dy = sig.min_y as i32 - y0 as i32;
                class = match (g0, dx, dy, period) {
                    (0, 0, 0, 1) => Class::StillLife,
                    (0, 0, 0, p) => Class::Oscillator { period: p },
                    (0, ..) => Class::Spaceship { period, dx, dy },
                    _ => Class::Settles { at: g0, period, dx, dy },
                };
                break;
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert((gen, sig.min_x, sig.min_y));
            }
        }

        if gen < budget.max_generations {
            grid.step();
        }
    }

    // No configuration cycle. Fall back to asking whether the *population*
    // became periodic, which is what "settles" means for anything that throws
    // off gliders.
    if class == Class::Unresolved {
        if let Some((at, period)) = population_settles(&pops) {
            class = Class::Stabilises { at, period };
        }
    }

    // No cycle found. If it was *still* rising when we stopped, that is the
    // signature of a gun, puffer, rake or breeder. Comparing windowed means
    // rather than single samples matters: a methuselah's population is wildly
    // noisy mid-chaos and any two points can say anything.
    if class == Class::Unresolved && pops.len() >= 20 {
        let n = pops.len();
        let mean = |s: &[u32]| s.iter().map(|&p| p as f64).sum::<f64>() / s.len().max(1) as f64;
        let tail = mean(&pops[n - n / 10..]);
        let mid = mean(&pops[n / 2 - n / 20..n / 2 + n / 20]);
        if tail > initial_population as f64 * 1.5 && tail > mid * 1.15 {
            class = Class::InfiniteGrowth;
        }
    }

    Analysis {
        class,
        initial_population,
        final_population: *pops.last().unwrap_or(&0),
        max_population,
        generations,
        reached_edge,
        grid: (grid.width(), grid.height()),
    }
}
