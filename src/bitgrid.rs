//! Bit-parallel Life.
//!
//! Cells are packed 64 to a `u64`: bit `i` of a word holds the cell at column
//! `word * 64 + i`. Every operation below acts on all 64 lanes at once, so the
//! rule is evaluated for 64 cells per instruction sequence with no branches.
//!
//! Counts are held as *bit-planes*: a value 0..=3 for 64 columns lives in two
//! words, one per bit position. Addition is then just a logic circuit, and the
//! bitwise ops run 64 copies of that circuit in parallel.

/// Sum of the three cells `(col-1, col, col+1)` for all 64 lanes of `cur`,
/// returned as two bit-planes: `.0` is bit 0 of the sum, `.1` is bit 1.
///
/// `prev` and `next` are the adjacent words in the same row; they supply the
/// two cells that fall outside `cur` when shifting.
#[inline(always)]
fn hsum(prev: u64, cur: u64, next: u64) -> (u64, u64) {
    // (cur << 1)[i] == cur[i-1], and column 0's left neighbour is prev's
    // bit 63. Mirror image for the right.
    let lft = (cur << 1) | (prev >> 63);
    let rgt = (cur >> 1) | (next << 63);

    // Full adder over lft + cur + rgt.
    let x = lft ^ cur;
    (x ^ rgt, (lft & cur) | (rgt & x))
}

/// The Life rule for one word, given the horizontal window sums of the row
/// above, this row, and the row below, plus this row's raw cells.
#[inline(always)]
fn rule(a: (u64, u64), m: (u64, u64), b: (u64, u64), mid: u64) -> u64 {
    // Add the three 2-bit numbers into a 4-bit total. This is S9: every cell
    // in the 3x3 box, centre included.
    let x = a.0 ^ m.0;
    let t0 = x ^ b.0;
    let c1 = (a.0 & m.0) | (b.0 & x); // weight 2

    let y = a.1 ^ m.1;
    let h0 = y ^ b.1; // weight 2
    let h1 = (a.1 & m.1) | (b.1 & y); // weight 4

    let t1 = c1 ^ h0;
    let c2 = c1 & h0; // weight 4
    let t2 = h1 ^ c2;
    let t3 = h1 & c2; // weight 8

    // Counting the centre lets the whole rule collapse to two equality tests:
    // a live cell needs 2 or 3 neighbours (S9 == 3 or 4), a dead cell needs
    // exactly 3 (S9 == 3).
    let is3 = !t3 & !t2 & t1 & t0;
    let is4 = !t3 & t2 & !t1 & !t0;
    is3 | (mid & is4)
}

/// What lies outside the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// Edges wrap. Pretty, but a gun's own gliders come back around and
    /// demolish it, so it is the wrong choice for showcasing patterns.
    Torus,
    /// Everything outside the grid is permanently dead. Gliders that leave
    /// simply leave.
    Dead,
}

/// A Life grid. Width is rounded up to a multiple of 64.
#[derive(Clone)]
pub struct BitGrid {
    width: usize,
    height: usize,
    stride: usize, // words per row
    boundary: Boundary,
    front: Vec<u64>,
    back: Vec<u64>,
    generation: u64,
}

/// A translation-invariant fingerprint of the live cells, used to detect when
/// a pattern has returned to a previous state — possibly somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signature {
    /// Hash of the live cells with the bounding box moved to the origin.
    pub hash: u64,
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
    pub population: u32,
}

impl Signature {
    /// The part that must match for two states to be the same pattern, with
    /// position deliberately excluded.
    pub fn key(&self) -> (u64, u32, u32, u32) {
        (self.hash, self.population, self.max_x - self.min_x, self.max_y - self.min_y)
    }
}

impl BitGrid {
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_boundary(width, height, Boundary::Torus)
    }

    pub fn with_boundary(width: usize, height: usize, boundary: Boundary) -> Self {
        // Below height 3 a cell is its own vertical neighbour on the torus,
        // which breaks the "S9 counts the centre exactly once" assumption the
        // rule is built on. Not worth supporting.
        assert!(width > 0, "grid must be non-empty");
        assert!(height >= 3, "grid must be at least 3 rows tall");
        let stride = width.div_ceil(64);
        let width = stride * 64;
        Self {
            width,
            height,
            stride,
            boundary,
            front: vec![0; stride * height],
            back: vec![0; stride * height],
            generation: 0,
        }
    }

    pub fn boundary(&self) -> Boundary {
        self.boundary
    }

    pub fn set_boundary(&mut self, boundary: Boundary) {
        self.boundary = boundary;
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

    /// Raw backing words, row-major, `width / 64` words per row.
    pub fn words(&self) -> &[u64] {
        &self.front
    }

    /// The previous generation, still sitting in the back buffer after the
    /// swap. All zeros before the first [`step`](Self::step). Free to read,
    /// which makes it a cheap source of "just died" cells for rendering.
    pub fn prev_words(&self) -> &[u64] {
        &self.back
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        debug_assert!(x < self.width && y < self.height);
        self.front[y * self.stride + x / 64] >> (x % 64) & 1 == 1
    }

    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        debug_assert!(x < self.width && y < self.height);
        let bit = 1u64 << (x % 64);
        let w = &mut self.front[y * self.stride + x / 64];
        if alive {
            *w |= bit;
        } else {
            *w &= !bit;
        }
    }

    pub fn clear(&mut self) {
        self.front.fill(0);
        self.generation = 0;
    }

    pub fn population(&self) -> u64 {
        self.front.iter().map(|w| w.count_ones() as u64).sum()
    }

    /// Advance one generation.
    pub fn step(&mut self) {
        let (h, s) = (self.height, self.stride);
        let wrap = self.boundary == Boundary::Torus;
        let front = &self.front;
        let back = &mut self.back;

        // Off-grid rows and columns read as zero, which is exactly the dead
        // boundary. On a torus they are never `None`.
        #[inline(always)]
        fn at(front: &[u64], row: Option<usize>, x: Option<usize>) -> u64 {
            match (row, x) {
                (Some(r), Some(i)) => front[r + i],
                _ => 0,
            }
        }

        for y in 0..h {
            let above = if y > 0 {
                Some((y - 1) * s)
            } else if wrap {
                Some((h - 1) * s)
            } else {
                None
            };
            let mid = y * s;
            let below = if y + 1 < h {
                Some((y + 1) * s)
            } else if wrap {
                Some(0)
            } else {
                None
            };

            for x in 0..s {
                // Word wrap doubles as column wrap: pulling bit 63 out of the
                // row's last word is exactly the torus edge.
                let xl = if x > 0 {
                    Some(x - 1)
                } else if wrap {
                    Some(s - 1)
                } else {
                    None
                };
                let xr = if x + 1 < s {
                    Some(x + 1)
                } else if wrap {
                    Some(0)
                } else {
                    None
                };
                let here = Some(x);

                let a = hsum(at(front, above, xl), at(front, above, here), at(front, above, xr));
                let m = hsum(at(front, Some(mid), xl), front[mid + x], at(front, Some(mid), xr));
                let b = hsum(at(front, below, xl), at(front, below, here), at(front, below, xr));

                back[mid + x] = rule(a, m, b, front[mid + x]);
            }
        }

        std::mem::swap(&mut self.front, &mut self.back);
        self.generation += 1;
    }

    /// Live cells in row-major order, reusing `out`'s allocation.
    pub fn live_cells_into(&self, out: &mut Vec<(u32, u32)>) {
        out.clear();
        for (i, &word) in self.front.iter().enumerate() {
            if word == 0 {
                continue;
            }
            let y = (i / self.stride) as u32;
            let base = ((i % self.stride) * 64) as u32;
            let mut w = word;
            while w != 0 {
                out.push((base + w.trailing_zeros(), y));
                w &= w - 1; // clear lowest set bit
            }
        }
    }

    /// Fingerprint the current state, normalised so that a pattern which has
    /// merely moved hashes the same as where it started.
    pub fn signature_into(&self, buf: &mut Vec<(u32, u32)>) -> Signature {
        self.live_cells_into(buf);
        if buf.is_empty() {
            return Signature::default();
        }
        // `buf` is ordered by row, then by column within a row.
        let min_y = buf[0].1;
        let max_y = buf[buf.len() - 1].1;
        let mut min_x = u32::MAX;
        let mut max_x = 0;
        for &(x, _) in buf.iter() {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }

        // FNV-1a over origin-relative coordinates.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for &(x, y) in buf.iter() {
            for v in [x - min_x, y - min_y] {
                hash ^= v as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        Signature { hash, min_x, min_y, max_x, max_y, population: buf.len() as u32 }
    }

    pub fn signature(&self) -> Signature {
        self.signature_into(&mut Vec::new())
    }

    pub fn step_n(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Fill with random cells. `density` is a percentage, 0..=100.
    pub fn randomize(&mut self, seed: u64, density: u32) {
        let mut rng = Rng::new(seed);
        let density = density.min(100) as u64;
        for y in 0..self.height {
            for x in 0..self.width {
                self.set(x, y, rng.next() % 100 < density);
            }
        }
        self.generation = 0;
    }

    /// Stamp a pattern's live cells with its top-left corner at `(ox, oy)`.
    /// Cells that land off-grid wrap on a torus and are dropped otherwise.
    /// Dead cells in the pattern are left alone, so stamps compose.
    pub fn stamp(&mut self, pattern: &crate::pattern::Pattern, ox: usize, oy: usize) {
        for &(px, py) in &pattern.live {
            let (x, y) = (ox + px, oy + py);
            match self.boundary {
                Boundary::Torus => self.set(x % self.width, y % self.height, true),
                Boundary::Dead => {
                    if x < self.width && y < self.height {
                        self.set(x, y, true);
                    }
                }
            }
        }
    }

    /// Stamp `pattern` in the middle of the grid, returning the offset used.
    pub fn stamp_centred(&mut self, pattern: &crate::pattern::Pattern) -> (usize, usize) {
        let ox = self.width.saturating_sub(pattern.width) / 2;
        let oy = self.height.saturating_sub(pattern.height) / 2;
        self.stamp(pattern, ox, oy);
        (ox, oy)
    }
}

/// xorshift64. Deterministic and dependency-free — this is for seeding soups
/// and reproducible tests, not for anything that cares about randomness.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x2545_F491_4F6C_DD1D } else { seed })
    }

    #[allow(clippy::should_implement_trait)] // it is a generator, not an iterator
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}
