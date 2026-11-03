//! FrameEase core library: everything except the GUI shell.
//!
//! Modules are split by responsibility: media probing (`media`), backend
//! discovery and capability detection (`tools`, `caps`), argument planning
//! (`plan`), process management (`process`), output verification (`verify`),
//! file safety (`fsx`), queue + persistence (`engine`, `store`, `job`) and the
//! lock-tolerant results manifest (`manifest`).

pub mod caps;
pub mod encoders;
pub mod engine;
pub mod fsx;
pub mod job;
pub mod manifest;
pub mod media;
pub mod plan;
pub mod presets;
pub mod process;
pub mod scan;
pub mod settings;
pub mod store;
pub mod tools;
pub mod trial;
pub mod verify;
