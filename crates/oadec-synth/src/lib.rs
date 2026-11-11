//! TrueHD and MLP streams written from a recipe.
//!
//! Part of the `oadec` object-audio decoder engine, and the part of it that
//! writes: a stream comes out of samples and a plan that says which element
//! of the syntax codes them. It exists because encoders choose, and what an
//! encoder never chooses no real stream carries; the census of a library
//! lists the elements nothing has ever been read with. A stream written here
//! carries the ones its recipe names, and its samples are known before any
//! decoder has seen it.
//!
//! The writer shares no code with the decoder it tests. [`write`] holds the
//! syntax and the arithmetic a second time, written from the encoding side.
//! What a stream written here is worth as a reference is settled outside
//! this crate: by decoders that share nothing with either.

pub mod bits;
pub mod recipe;
pub mod sha256;
pub mod signal;
pub mod write;

pub use recipe::{Made, RECIPES, Recipe};
pub use write::{BlockPlan, Coding, Family, Filter, Layout, Stream, UnitPlan, write};
