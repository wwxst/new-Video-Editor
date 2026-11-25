//! Arcline renderer (docs/03): built-in effect manifests, pure frame planning, and the wgpu
//! direct compositor. Depends on `arc-core` only — never on FFmpeg or UI.

pub mod builtin;
pub mod plan;
pub mod renderer;

pub use renderer::{DrawItem, DrawLayer, FrameReport, Image, RenderError, Renderer};
