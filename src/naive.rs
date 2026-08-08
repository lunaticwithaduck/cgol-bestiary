//! One byte per cell, eight neighbours counted one at a time.
//!
//! This is deliberately the dumbest correct implementation. It is the oracle
//! the bit-parallel engine is diffed against — if the two ever disagree, the
//! clever one is wrong.

#[derive(Clone)]
pub struct NaiveGrid {
    width: usize,
    height: usize,
    front: Vec<u8>,
    back: Vec<u8>,
    generation: u64,
}

impl NaiveGrid {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            front: vec![0; width * height],
            back: vec![0; width * height],
            generation: 0,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.front[y * self.width + x] != 0
    }

    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        self.front[y * self.width + x] = alive as u8;
    }

    pub fn population(&self) -> u64 {
        self.front.iter().map(|&c| c as u64).sum()
    }

    pub fn step(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                let mut n = 0;
                for dy in [self.height - 1, 0, 1] {
                    for dx in [self.width - 1, 0, 1] {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = (x + dx) % self.width;
                        let ny = (y + dy) % self.height;
                        n += self.front[ny * self.width + nx];
                    }
                }
                let alive = self.front[y * self.width + x] != 0;
                self.back[y * self.width + x] = ((n == 3) || (alive && n == 2)) as u8;
            }
        }
        std::mem::swap(&mut self.front, &mut self.back);
        self.generation += 1;
    }

    pub fn step_n(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }
}
