//! Evolving Worlds simulation core: typed rule language, compiler, and
//! deterministic headless runtime. This crate never depends on a renderer.

#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::result_large_err
)]

pub mod assets;
pub mod biology;
pub mod commands;
pub mod compiler;
pub mod eval;
pub mod grid;
pub mod ir;
pub mod optimize;
pub mod resolver;
pub mod rng;
pub mod schema;
pub mod state;
pub mod units;
pub mod weather;
pub mod world;
pub mod worldgen;

pub use world::{RunConfig, World};
