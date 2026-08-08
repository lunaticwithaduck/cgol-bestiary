//! `cargo run --release --bin bench`
//!
//! Answers the only question that matters about the bit tricks: how much did
//! they actually buy over counting neighbours one at a time?

use conway::{BitGrid, NaiveGrid};
use std::time::Instant;

fn rate(cells: usize, gens: usize, secs: f64) -> String {
    let per_sec = (cells * gens) as f64 / secs;
    format!("{:>8.2} M cell-updates/sec", per_sec / 1e6)
}

fn main() {
    {
        let (w, h, gens) = (512, 512, 20);
        let mut g = NaiveGrid::new(w, h);
        let mut r = conway::bitgrid::Rng::new(42);
        for y in 0..h {
            for x in 0..w {
                g.set(x, y, r.next() % 100 < 32);
            }
        }
        let t = Instant::now();
        g.step_n(gens);
        let secs = t.elapsed().as_secs_f64();
        println!("naive  {w}x{h} x{gens:<5} {:>7.3}s  {}", secs, rate(w * h, gens, secs));
    }

    for (w, h, gens) in [(512usize, 512usize, 200usize), (2048, 2048, 100)] {
        let mut g = BitGrid::new(w, h);
        g.randomize(42, 32);
        let t = Instant::now();
        g.step_n(gens);
        let secs = t.elapsed().as_secs_f64();
        println!("bitwise {w}x{h} x{gens:<4} {:>7.3}s  {}", secs, rate(w * h, gens, secs));
    }
}
