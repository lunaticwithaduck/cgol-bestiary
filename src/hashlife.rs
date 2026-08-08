//! HashLife backend, for the patterns the bitmap engine cannot represent.
//!
//! Wraps our vendored [`golback`], a quadtree/hash-consing engine.
//!
//! Nothing on the hot path costs population. Rendering **walks the quadtree**,
//! descending only into nodes that intersect the viewport and stopping below a
//! pixel, so it costs visible-node count. Population comes straight off the
//! root node, and the bounding box from a memoised tree walk that costs
//! distinct-node count. `to_coords` — which materialises every live cell — is
//! now only reachable through [`HashWorld::snapshot_cells`], used by the tests
//! to diff the quadtree walk against a naive rasteriser.
//!
//! Two things upstream does not do, both handled here:
//!
//! * **Loading still flattens.** `from_coords` wants every coordinate, so
//!   [`MAX_CELLS`] stands: `metapixel-parity64` would need 1.6GB of pairs to
//!   load a pattern its own file describes in 5,572 nodes. Building the tree
//!   directly from a macrocell DAG would remove this.
//! * **The universe did not grow itself.** `centre` wraps a node in a larger
//!   one, but `advance_aux` undoes that with `successor`, so the level never
//!   rose and a travelling pattern eventually decayed against the wall in
//!   silence. `Universe::ensure_room_for` fixes that upstream, so a pattern can
//!   now run until `i64` coordinates run out.

use crate::macrocell::Macrocell;
use golback::universe::Universe;

/// Above this, `from_coords` needs more memory than a wasm heap should be
/// asked for. 128 million cells would want two gigabytes of coordinate pairs.
pub const MAX_CELLS: u128 = 8_000_000;

/// Two levels of slack so the pattern starts inside the central quarter. It no
/// longer needs generous headroom: the universe grows itself now, and a
/// shallower initial tree makes `from_coords` cheaper.
const HEADROOM_LEVELS: u32 = 2;

const SCRATCH: usize = 1 << 20;

const ALIVE: [u8; 4] = [0x7e, 0xe7, 0x87, 0xff];
const DEAD: [u8; 4] = [0x0b, 0x0e, 0x14, 0xff];

/// Camera geometry for one frame.
struct View {
    cells_per_pixel: i64,
    pixels_per_cell: i64,
    cam_x: i64,
    cam_y: i64,
    span_x: i64,
    span_y: i64,
}

/// Paint a `size`x`size` square of live cells at pixel `(px, py)`, clipped.
fn fill(rgba: &mut [u8], w: usize, h: usize, px: i64, py: i64, size: i64) {
    if px + size <= 0 || py + size <= 0 || px >= w as i64 || py >= h as i64 {
        return;
    }
    for y in py.max(0)..(py + size).min(h as i64) {
        let row = y as usize * w * 4;
        for x in px.max(0)..(px + size).min(w as i64) {
            let i = row + x as usize * 4;
            rgba[i..i + 4].copy_from_slice(&ALIVE);
        }
    }
}

pub struct HashWorld {
    universe: Universe,
    /// Live cells, filled in only by `render_from_cells` for the tests.
    cells: Vec<(i64, i64)>,
    population: usize,
    rgba: Vec<u8>,
    scratch: Vec<u8>,
    bbox: (i64, i64, i64, i64),
    generation: f64,
    loaded: bool,
    /// Where the pattern's bounding box started, for reporting displacement.
    origin: (i64, i64),
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
            population: 0,
            rgba: Vec::new(),
            scratch: vec![0; SCRATCH],
            bbox: (0, 0, 0, 0),
            generation: 0.0,
            loaded: false,
            origin: (0, 0),
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

        // Just enough to hold the pattern with it centred. Growth beyond this
        // is the universe's own job now — see `Universe::ensure_room_for`.
        let needed = (128 - (span - 1).leading_zeros()).max(4);
        let level = needed + HEADROOM_LEVELS;

        self.universe = Universe::new();
        self.universe.init(level);
        let owned: Vec<(i64, i64)> = cells.to_vec();
        self.universe.from_coords(&owned);
        self.origin = (x0, y0);
        self.generation = 0.0;
        self.loaded = true;
        self.refresh();
        true
    }

    /// True once the universe has grown as far as `i64` coordinates allow, past
    /// which cells could be lost. A level-60 universe is 2^60 cells square, so
    /// in practice this never fires — it is a backstop, not a limit to plan
    /// around. Kept because silently losing cells is the one failure mode worth
    /// refusing to have.
    pub fn clipped(&self) -> bool {
        self.loaded && self.universe.at_growth_limit()
    }

    /// Current universe level; grows as the pattern travels.
    pub fn level(&self) -> u32 {
        self.universe.level()
    }

    /// Pull every live cell out of the quadtree.
    ///
    /// Costs population, unlike everything else on the hot path, so it is not
    /// used for rendering or stepping — only by the tests and by
    /// [`render_from_cells`](Self::render_from_cells).
    pub fn snapshot_cells(&mut self) -> &[(i64, i64)] {
        self.cells = self.universe.to_coords().into_iter().collect();
        &self.cells
    }

    pub fn step(&mut self, n: u64) {
        if !self.loaded || n == 0 || self.clipped() {
            return;
        }
        // `advance` grows the universe first, so nothing escapes.
        self.universe.advance(n);
        self.generation += n as f64;
        self.refresh();
    }

    /// Recompute the bounding box and population from the quadtree.
    ///
    /// Both come from the tree rather than from a cell list, so stepping costs
    /// distinct-node count rather than population. `to_coords` is now only
    /// touched by [`render_from_cells`](Self::render_from_cells), which exists
    /// for the tests to diff the quadtree walk against.
    fn refresh(&mut self) {
        self.population = self.universe.population();
        self.bbox = self.universe.bounds().unwrap_or((0, 0, 0, 0));
    }

    pub fn population(&self) -> f64 {
        self.population as f64
    }

    pub fn generation(&self) -> f64 {
        self.generation
    }

    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        self.bbox
    }

    /// Geometry shared by both renderers, derived from the camera and zoom.
    ///
    /// `cells_per_pixel` is forced to a power of two and the camera is snapped
    /// to a multiple of it. Both are needed for the quadtree walk to be exact:
    /// nodes are power-of-two sized and aligned, so with an arbitrary ratio or
    /// an unsnapped camera a node can straddle two pixels and the walk has no
    /// way to decide which one to light.
    fn view(&self, cam_x: f64, cam_y: f64, scale: i32, w: usize, h: usize) -> View {
        let zoom = if scale == 0 { 1 } else { scale };
        let cells_per_pixel = if zoom > 0 {
            1
        } else {
            let k = (-zoom) as i64;
            1i64 << (63 - k.leading_zeros())
        };
        let pixels_per_cell = if zoom > 0 { zoom as i64 } else { 1 };
        View {
            cells_per_pixel,
            pixels_per_cell,
            cam_x: (cam_x.floor() as i64).div_euclid(cells_per_pixel) * cells_per_pixel,
            cam_y: (cam_y.floor() as i64).div_euclid(cells_per_pixel) * cells_per_pixel,
            // Round up: a partial pixel at the edge still needs its cells.
            // `i64::div_ceil` is unstable, and both values are positive here.
            span_x: (w as i64 + pixels_per_cell - 1) / pixels_per_cell * cells_per_pixel,
            span_y: (h as i64 + pixels_per_cell - 1) / pixels_per_cell * cells_per_pixel,
        }
    }

    fn begin_frame(&mut self, w: usize, h: usize) {
        let needed = w * h * 4;
        if self.rgba.len() != needed {
            self.rgba.resize(needed, 0);
        }
        for px in self.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&DEAD);
        }
    }

    /// Draw a window of the universe into an RGBA buffer by **walking the
    /// quadtree**, descending only into nodes that intersect the viewport and
    /// stopping at anything smaller than a pixel.
    ///
    /// Cost is proportional to the number of visible nodes, so it is unaffected
    /// by population — which is the whole point. Camera coordinates are `f64`
    /// because a level-44 universe runs far past what an `i32` can address.
    pub fn render(&mut self, cam_x: f64, cam_y: f64, scale: i32, w: usize, h: usize) -> *const u8 {
        self.begin_frame(w, h);
        if !self.loaded || w == 0 || h == 0 {
            return self.rgba.as_ptr();
        }
        let v = self.view(cam_x, cam_y, scale, w, h);

        // Disjoint field borrows: the walk reads `universe`, the closure writes
        // `rgba`.
        let universe = &self.universe;
        let rgba = &mut self.rgba;

        universe.visit_region(
            v.cam_x,
            v.cam_y,
            v.cam_x + v.span_x - 1,
            v.cam_y + v.span_y - 1,
            v.cells_per_pixel,
            &mut |left, bottom, side, _population| {
                let px0 = (left - v.cam_x).div_euclid(v.cells_per_pixel) * v.pixels_per_cell;
                let py0 = (bottom - v.cam_y).div_euclid(v.cells_per_pixel) * v.pixels_per_cell;
                let size = (side / v.cells_per_pixel).max(1) * v.pixels_per_cell;
                fill(rgba, w, h, px0, py0, size);
            },
        );
        self.rgba.as_ptr()
    }

    /// The previous renderer: plot every cached live cell. Kept because it is
    /// obviously correct and independent of the quadtree walk, which lets the
    /// tests diff one against the other.
    pub fn render_from_cells(
        &mut self,
        cam_x: f64,
        cam_y: f64,
        scale: i32,
        w: usize,
        h: usize,
    ) -> &[u8] {
        self.begin_frame(w, h);
        if self.loaded && w > 0 && h > 0 {
            self.snapshot_cells();
            let v = self.view(cam_x, cam_y, scale, w, h);
            let cells = &self.cells;
            let rgba = &mut self.rgba;
            for &(x, y) in cells {
                let px = (x - v.cam_x).div_euclid(v.cells_per_pixel) * v.pixels_per_cell;
                let py = (y - v.cam_y).div_euclid(v.cells_per_pixel) * v.pixels_per_cell;
                fill(rgba, w, h, px, py, v.pixels_per_cell);
            }
        }
        &self.rgba
    }

    pub fn render_len(&self) -> u32 {
        self.rgba.len() as u32
    }

    /// The frame buffer as last rendered.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
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
