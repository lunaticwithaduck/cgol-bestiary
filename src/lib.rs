//! Bit-parallel Conway's Game of Life.
//!
//! The interesting code is in [`bitgrid`]. [`naive`] exists purely so the
//! tests have something obviously-correct to diff against, and [`analysis`]
//! uses the engine to work out what a pattern is by running it.

pub mod analysis;
pub mod bitgrid;
pub mod ffi;
pub mod hashlife;
pub mod macrocell;
pub mod naive;
pub mod pattern;

pub use analysis::{analyse, Analysis, Budget, Class};
pub use bitgrid::{BitGrid, Boundary, Signature};
pub use hashlife::HashWorld;
pub use macrocell::Macrocell;
pub use naive::NaiveGrid;
pub use pattern::{Pattern, Rule};
