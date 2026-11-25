// Deep `im` collections (tracks/clips/effects) overflow rustc's default obligation depth when a
// `Project` is moved into the preview worker (the Send check); 512 is enough and costs compile time.
#![recursion_limit = "512"]
//! Arcline session layer: inspection views, the export job, the CLI, and (from Phase F)
//! JSON-RPC and MCP.

pub mod export;
pub mod inspect;
pub mod media;
pub mod preview;
pub mod preview_scheduler;
pub mod rpc;
