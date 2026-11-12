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
//!
//! [`ac3`] does the same for a narrow part of AC-3: syncframes of two
//! channels whose delta bit allocation moves the masking curve, which the
//! encoders in use never do. Their samples are known to the accuracy of
//! floating point, and [`ac3_recipe`] names the streams.

pub mod ac3;
pub mod ac3_recipe;
pub mod ac3_tables;
pub mod bits;
pub mod recipe;
pub mod sha256;
pub mod signal;
pub mod write;

pub use ac3_recipe::{AC3_PROBES, AC3_RECIPES, Ac3Probe, Ac3Recipe};
pub use recipe::{Made, PROBES, Probe, RECIPES, Recipe, pcm_bytes};
pub use write::{
    Above, BlockPlan, Coding, Family, Filter, Layout, Matrix, Protection, Stream, UnitPlan, write,
};
