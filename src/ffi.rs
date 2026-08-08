//! Raw C ABI for the browser. No wasm-bindgen — the JS side reads linear
//! memory directly, which keeps the module small and readable in `wasm2wat`.
//!
//! Rendering is viewport-based rather than whole-grid: a 4096² grid would be a
//! 67MB RGBA buffer per frame, while the visible area never exceeds a couple
//! of megapixels no matter how big the world is.

use crate::bitgrid::Boundary;
use crate::pattern::Pattern;
use crate::BitGrid;

const SCRATCH: usize = 1 << 20; // 1 MB — the largest RLE in the archive fits

pub struct World {
    grid: BitGrid,
    rgba: Vec<u8>,
    scratch: Vec<u8>,
    /// Where the last stamped pattern landed: x, y, w, h.
    stamp: [u32; 4],
    cells: Vec<(u32, u32)>,
}

/// # Safety
/// `w` must come from [`conway_new`] and not yet have been freed.
unsafe fn world<'a>(w: *mut World) -> &'a mut World {
    &mut *w
}

fn boundary_of(code: u32) -> Boundary {
    if code == 0 {
        Boundary::Torus
    } else {
        Boundary::Dead
    }
}

#[no_mangle]
pub extern "C" fn conway_new(width: u32, height: u32, boundary: u32) -> *mut World {
    let grid = BitGrid::with_boundary(width as usize, height.max(3) as usize, boundary_of(boundary));
    Box::into_raw(Box::new(World {
        grid,
        rgba: Vec::new(),
        scratch: vec![0; SCRATCH],
        stamp: [0; 4],
        cells: Vec::new(),
    }))
}

/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_free(w: *mut World) {
    if !w.is_null() {
        drop(Box::from_raw(w));
    }
}

/// Replace the grid with an empty one of a new size. Cheaper than tearing the
/// whole world down, and keeps the scratch and render buffers.
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_resize(w: *mut World, width: u32, height: u32, boundary: u32) {
    let w = world(w);
    w.grid = BitGrid::with_boundary(width as usize, height.max(3) as usize, boundary_of(boundary));
    w.stamp = [0; 4];
}

/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_step(w: *mut World, n: u32) {
    world(w).grid.step_n(n as usize);
}

/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_clear(w: *mut World) {
    world(w).grid.clear();
}

/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_randomize(w: *mut World, seed: u32, density: u32) {
    world(w).grid.randomize(seed as u64, density);
}

/// Out-of-range coordinates are ignored, so JS never has to clamp.
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_set(w: *mut World, x: i32, y: i32, alive: u32) {
    let g = &mut world(w).grid;
    if x >= 0 && y >= 0 && (x as usize) < g.width() && (y as usize) < g.height() {
        g.set(x as usize, y as usize, alive != 0);
    }
}

macro_rules! getter {
    ($name:ident, $ty:ty, $body:expr) => {
        /// # Safety
        /// See [`world`].
        #[no_mangle]
        pub unsafe extern "C" fn $name(w: *mut World) -> $ty {
            let f: fn(&mut World) -> $ty = $body;
            f(world(w))
        }
    };
}

getter!(conway_width, u32, |w| w.grid.width() as u32);
getter!(conway_height, u32, |w| w.grid.height() as u32);
getter!(conway_generation, f64, |w| w.grid.generation() as f64);
getter!(conway_population, f64, |w| w.grid.population() as f64);
getter!(conway_stamp_x, u32, |w| w.stamp[0]);
getter!(conway_stamp_y, u32, |w| w.stamp[1]);
getter!(conway_stamp_w, u32, |w| w.stamp[2]);
getter!(conway_stamp_h, u32, |w| w.stamp[3]);

/// Buffer JS writes RLE text into before calling [`conway_load_rle`].
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_scratch_ptr(w: *mut World) -> *mut u8 {
    world(w).scratch.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn conway_scratch_cap() -> u32 {
    SCRATCH as u32
}

/// Parse `len` bytes of UTF-8 RLE from the scratch buffer and stamp it in the
/// middle of the grid. Returns 1 on success, 0 if it did not parse or is not
/// Conway's rule.
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_load_rle(w: *mut World, len: u32) -> u32 {
    let w = world(w);
    let len = (len as usize).min(w.scratch.len());
    let Ok(text) = std::str::from_utf8(&w.scratch[..len]) else {
        return 0;
    };
    let Ok(p) = Pattern::parse_rle(text) else {
        return 0;
    };
    if !p.is_life() {
        return 0;
    }
    let (ox, oy) = w.grid.stamp_centred(&p);
    w.stamp = [ox as u32, oy as u32, p.width as u32, p.height as u32];
    1
}

/// Bounding box of the live cells, written into the scratch buffer as four
/// little-endian `u32`s (min_x, min_y, max_x, max_y). Returns the population.
/// Not cheap — call it when fitting the view, not every frame.
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_live_bbox(w: *mut World) -> f64 {
    let w = world(w);
    let sig = w.grid.signature_into(&mut w.cells);
    for (i, v) in [sig.min_x, sig.min_y, sig.max_x, sig.max_y].iter().enumerate() {
        w.scratch[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    sig.population as f64
}

const ALIVE: [u8; 4] = [0x7e, 0xe7, 0x87, 0xff];
const DYING: [u8; 4] = [0x22, 0x4a, 0x36, 0xff];
const DEAD: [u8; 4] = [0x0b, 0x0e, 0x14, 0xff];
const GRID: [u8; 4] = [0x14, 0x19, 0x22, 0xff];
const VOID: [u8; 4] = [0x07, 0x09, 0x0d, 0xff]; // outside the world

/// Paint a window of the world into an RGBA buffer and return a pointer to it.
///
/// `scale > 0` draws each cell as `scale`×`scale` pixels; `scale < 0` packs
/// `-scale`×`-scale` cells into each pixel, lit if any of them is alive.
/// `(cam_x, cam_y)` is the cell at the top-left of the viewport and may be
/// negative — the area outside the world is drawn distinctly.
///
/// Cells that died on the previous generation are drawn dim, which is free:
/// the previous generation is still in the back buffer after the swap.
///
/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_render_view(
    w: *mut World,
    cam_x: i32,
    cam_y: i32,
    scale: i32,
    out_w: u32,
    out_h: u32,
) -> *const u8 {
    let w = world(w);
    let (ow, oh) = (out_w as usize, out_h as usize);
    let needed = ow * oh * 4;
    if w.rgba.len() != needed {
        w.rgba.resize(needed, 0);
    }

    let (gw, gh) = (w.grid.width() as i64, w.grid.height() as i64);
    let now = w.grid.words();
    let prev = w.grid.prev_words();
    let stride = (w.grid.width() / 64) as i64;

    let live = |words: &[u64], x: i64, y: i64| -> bool {
        words[(y * stride + (x >> 6)) as usize] >> (x & 63) & 1 == 1
    };

    let zoom = if scale == 0 { 1 } else { scale };
    // Only draw cell separators once cells are big enough for it to read as a
    // grid rather than as noise.
    let gridlines = zoom >= 6;

    for py in 0..oh {
        let (cy, sy) = if zoom > 0 {
            (cam_y as i64 + (py as i64 / zoom as i64), py as i64 % zoom as i64)
        } else {
            (cam_y as i64 + py as i64 * (-zoom) as i64, 0)
        };
        let row = py * ow * 4;

        for px in 0..ow {
            let (cx, sx) = if zoom > 0 {
                (cam_x as i64 + (px as i64 / zoom as i64), px as i64 % zoom as i64)
            } else {
                (cam_x as i64 + px as i64 * (-zoom) as i64, 0)
            };

            let colour = if cx < 0 || cy < 0 || cx >= gw || cy >= gh {
                VOID
            } else if gridlines && (sx == zoom as i64 - 1 || sy == zoom as i64 - 1) {
                GRID
            } else if zoom > 0 {
                if live(now, cx, cy) {
                    ALIVE
                } else if live(prev, cx, cy) {
                    DYING
                } else {
                    DEAD
                }
            } else {
                // Zoomed out: a pixel is lit if anything in its block is.
                let k = (-zoom) as i64;
                let (x1, y1) = ((cx + k).min(gw), (cy + k).min(gh));
                let mut any_now = false;
                let mut any_prev = false;
                'block: for y in cy.max(0)..y1 {
                    for x in cx.max(0)..x1 {
                        if live(now, x, y) {
                            any_now = true;
                            break 'block;
                        }
                        any_prev |= live(prev, x, y);
                    }
                }
                if any_now {
                    ALIVE
                } else if any_prev {
                    DYING
                } else {
                    DEAD
                }
            };

            let i = row + px * 4;
            w.rgba[i..i + 4].copy_from_slice(&colour);
        }
    }

    w.rgba.as_ptr()
}

/// # Safety
/// See [`world`].
#[no_mangle]
pub unsafe extern "C" fn conway_render_len(w: *mut World) -> u32 {
    world(w).rgba.len() as u32
}
