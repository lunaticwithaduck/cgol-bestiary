//! HashLife backend, for the patterns the bitmap engine cannot represent.
//!
//! Wraps [`golback`], a quadtree/hash-consing engine. Two properties of its
//! API shape everything here:
//!
//! * **The way in and the way out are both flat lists of cells.**
//!   `from_coords` takes every coordinate and `to_coords` returns every
//!   coordinate, so both cost population rather than node count. That imposes
//!   [`MAX_CELLS`] — `metapixel-parity64` would need 1.6GB just for the input
//!   vector.
//! * **`is_alive` is far too slow to render with**, at roughly 290ms per
//!   megapixel. So rendering rasterises a cached cell list instead: O(pixels)
//!   to clear plus O(population) to plot, rather than O(pixels) tree descents.
//!
//! The universe also does not grow itself — undersize it and escaping gliders
//! are silently lost — so [`HashWorld::load`] sizes it with headroom up front.

use crate::macrocell::Macrocell;
use golback::universe::Universe;

/// Above this, `from_coords` needs more memory than a wasm heap should be
/// asked for. 128 million cells would want two gigabytes of coordinate pairs.
pub const MAX_CELLS: u128 = 8_000_000;

/// Extra quadtree levels beyond what the pattern's bounding box needs, to give
/// travelling and growing patterns somewhere to go. Six levels is 64× the
/// pattern's own span in each direction.
const HEADROOM_LEVELS: u32 = 6;
/// Even a 3x3 glider gets a universe of 2^22 cells square — about a million
/// cells of travel, or four million generations, before it nears an edge.
const MIN_LEVEL: u32 = 22;
const MAX_LEVEL: u32 = 44;

const SCRATCH: usize = 1 << 20;

pub struct HashWorld {
    universe: Universe,
    /// Snapshot of the live cells, refreshed only when the universe advances.
    cells: Vec<(i64, i64)>,
    rgba: Vec<u8>,
    scratch: Vec<u8>,
    bbox: (i64, i64, i64, i64),
    generation: f64,
    loaded: bool,
    /// Quadtree level of the universe; it spans `2^level` cells square.
    level: u32,
    /// Where the pattern's bounding box started, for drift measurement.
    origin: (i64, i64),
    clipped: bool,
}

/// Why a load failed, as reported to JS.
#[repr(u32)]
pub enum LoadResult {
    Failed = 0,
    Ok = 1,
    NotLife = 2,
    TooManyCells = 3,
}

impl Default for HashWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl HashWorld {
    pub fn new() -> Self {
        Self {
            universe: Universe::new(),
            cells: Vec::new(),
            rgba: Vec::new(),
            scratch: vec![0; SCRATCH],
            bbox: (0, 0, 0, 0),
            generation: 0.0,
            loaded: false,
            level: 0,
            origin: (0, 0),
            clipped: false,
        }
    }

    pub fn load_macrocell(&mut self, text: &str) -> LoadResult {
        let Ok(m) = Macrocell::parse(text) else {
            return LoadResult::Failed;
        };
        if !m.is_life() {
            return LoadResult::NotLife;
        }
        if m.population > MAX_CELLS {
            return LoadResult::TooManyCells;
        }
        let Some(cells) = m.live_cells() else {
            return LoadResult::Failed;
        };
        if self.load_cells(&cells) {
            LoadResult::Ok
        } else {
            LoadResult::Failed
        }
    }

    /// Load an explicit cell list. Used by [`load_macrocell`](Self::load_macrocell)
    /// and by the tests, which cross-check this engine against the bitmap one.
    pub fn load_cells(&mut self, cells: &[(i64, i64)]) -> bool {
        if cells.is_empty() {
            return false;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &(x, y) in cells {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let span = ((x1 - x0 + 1).max(y1 - y0 + 1) as u128).max(1);

        // Room for the pattern *and for wherever it travels*, which is the part
        // that is easy to get wrong. golback never grows the universe, so
        // anything that leaves is silently lost: sized to the bounding box plus
        // a little, a glider decays against the wall after 128 cells, and an
        // undersized Gosper gun reports 564 cells where the truth is 11,144.
        //
        // Empty quadtree nodes are shared, so a vastly oversized universe costs
        // a handful of nodes and a little recursion depth. Being generous is
        // close to free; being tight is quietly wrong.
        let needed = (128 - (span - 1).leading_zeros()).max(4);
        let level = (needed + HEADROOM_LEVELS).clamp(MIN_LEVEL, MAX_LEVEL);

        self.universe = Universe::new();
        self.universe.init(level);
        let owned: Vec<(i64, i64)> = cells.to_vec();
        self.universe.from_coords(&owned);
        self.level = level;
        self.origin = (x0, y0);
        self.generation = 0.0;
        self.loaded = true;
        self.clipped = false;
        self.refresh();
        true
    }

    /// How far the pattern may drift from where it started before we stop
    /// trusting the result. Deliberately conservative: a quarter of the
    /// universe, rather than trying to pin down golback's exact bounds.
    fn safe_radius(&self) -> i64 {
        if self.level >= 4 {
            1i64 << (self.level - 2).min(62)
        } else {
            8
        }
    }

    /// True once the pattern has wandered far enough that cells may have been
    /// lost off the edge of the universe. Results past this point are an
    /// artefact of the box, so [`step`](Self::step) refuses to go further.
    pub fn clipped(&self) -> bool {
        self.clipped
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    /// Live cells, as last snapshotted from the quadtree.
    pub fn cells(&self) -> &[(i64, i64)] {
        &self.cells
    }

    pub fn step(&mut self, n: u64) {
        if !self.loaded || n == 0 || self.clipped {
            return;
        }
        self.universe.advance(n);
        self.generation += n as f64;
        self.refresh();

        // Stop the moment the pattern could be losing cells off the edge —
        // past that point we would be showing an artefact of the box, which is
        // worse than showing nothing.
        let r = self.safe_radius();
        let (x0, y0, x1, y1) = self.bbox;
        let (ox, oy) = self.origin;
        if (x0 - ox).abs() > r || (y0 - oy).abs() > r || (x1 - ox).abs() > r || (y1 - oy).abs() > r {
            self.clipped = true;
        }
    }

    /// Pull the cell list out of the quadtree and recompute the bounding box.
    fn refresh(&mut self) {
        self.cells = self.universe.to_coords().into_iter().collect();
        self.bbox = match self.cells.first() {
            None => (0, 0, 0, 0),
            Some(&(x, y)) => {
                let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
                for &(x, y) in &self.cells {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                (x0, y0, x1, y1)
            }
        };
    }

    pub fn population(&self) -> f64 {
        self.cells.len() as f64
    }

    pub fn generation(&self) -> f64 {
        self.generation
    }

    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        self.bbox
    }

    /// Rasterise the cached cells into an RGBA viewport.
    ///
    /// `scale > 0` draws each cell as `scale`×`scale` pixels; `scale < 0`
    /// packs `-scale` cells into each pixel. Camera coordinates are `f64`
    /// because a level-26 universe runs past what an `i32` can address.
    pub fn render(&mut self, cam_x: f64, cam_y: f64, scale: i32, w: usize, h: usize) -> *const u8 {
        const ALIVE: [u8; 4] = [0x7e, 0xe7, 0x87, 0xff];
        const DEAD: [u8; 4] = [0x0b, 0x0e, 0x14, 0xff];

        let needed = w * h * 4;
        if self.rgba.len() != needed {
            self.rgba.resize(needed, 0);
        }
        for px in self.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&DEAD);
        }

        let zoom = if scale == 0 { 1 } else { scale };
        let (cx, cy) = (cam_x.floor() as i64, cam_y.floor() as i64);

        for &(x, y) in &self.cells {
            let (dx, dy) = (x - cx, y - cy);
            let (px, py, size) = if zoom > 0 {
                (dx * zoom as i64, dy * zoom as i64, zoom as i64)
            } else {
                let k = (-zoom) as i64;
                // div_euclid so cells left of the camera floor correctly
                // rather than truncating toward zero.
                (dx.div_euclid(k), dy.div_euclid(k), 1)
            };
            if px + size <= 0 || py + size <= 0 || px >= w as i64 || py >= h as i64 {
                continue;
            }
            for oy in py.max(0)..(py + size).min(h as i64) {
                let row = oy as usize * w * 4;
                for ox in px.max(0)..(px + size).min(w as i64) {
                    let i = row + ox as usize * 4;
                    self.rgba[i..i + 4].copy_from_slice(&ALIVE);
                }
            }
        }
        self.rgba.as_ptr()
    }

    pub fn render_len(&self) -> u32 {
        self.rgba.len() as u32
    }

    pub fn scratch(&mut self) -> &mut [u8] {
        &mut self.scratch
    }
}

// ------------------------------------------------------------------ FFI ----

/// # Safety
/// `w` must come from [`conway_hl_new`] and not yet have been freed.
unsafe fn hw<'a>(w: *mut HashWorld) -> &'a mut HashWorld {
    &mut *w
}

#[no_mangle]
pub extern "C" fn conway_hl_new() -> *mut HashWorld {
    Box::into_raw(Box::new(HashWorld::new()))
}

/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_free(w: *mut HashWorld) {
    if !w.is_null() {
        drop(Box::from_raw(w));
    }
}

#[no_mangle]
pub extern "C" fn conway_hl_max_cells() -> f64 {
    MAX_CELLS as f64
}

/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_scratch_ptr(w: *mut HashWorld) -> *mut u8 {
    hw(w).scratch().as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn conway_hl_scratch_cap() -> u32 {
    SCRATCH as u32
}

/// Parse `len` bytes of macrocell text from the scratch buffer.
/// Returns a [`LoadResult`] discriminant.
///
/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_load(w: *mut HashWorld, len: u32) -> u32 {
    let w = hw(w);
    let len = (len as usize).min(w.scratch.len());
    let Ok(text) = std::str::from_utf8(&w.scratch[..len]) else {
        return LoadResult::Failed as u32;
    };
    let text = text.to_string();
    w.load_macrocell(&text) as u32
}

/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_step(w: *mut HashWorld, n: f64) {
    hw(w).step(n.max(0.0) as u64);
}

macro_rules! hl_getter {
    ($name:ident, $body:expr) => {
        /// # Safety
        /// See [`hw`].
        #[no_mangle]
        pub unsafe extern "C" fn $name(w: *mut HashWorld) -> f64 {
            let f: fn(&mut HashWorld) -> f64 = $body;
            f(hw(w))
        }
    };
}

hl_getter!(conway_hl_population, |w| w.population());
hl_getter!(conway_hl_generation, |w| w.generation());
hl_getter!(conway_hl_level, |w| w.level() as f64);
hl_getter!(conway_hl_clipped, |w| if w.clipped() { 1.0 } else { 0.0 });
hl_getter!(conway_hl_min_x, |w| w.bbox().0 as f64);
hl_getter!(conway_hl_min_y, |w| w.bbox().1 as f64);
hl_getter!(conway_hl_max_x, |w| w.bbox().2 as f64);
hl_getter!(conway_hl_max_y, |w| w.bbox().3 as f64);

/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_render_view(
    w: *mut HashWorld,
    cam_x: f64,
    cam_y: f64,
    scale: i32,
    out_w: u32,
    out_h: u32,
) -> *const u8 {
    hw(w).render(cam_x, cam_y, scale, out_w as usize, out_h as usize)
}

/// # Safety
/// See [`hw`].
#[no_mangle]
pub unsafe extern "C" fn conway_hl_render_len(w: *mut HashWorld) -> u32 {
    hw(w).render_len()
}
