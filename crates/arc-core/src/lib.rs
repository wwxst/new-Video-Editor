//! Arcline core: exact time, the document model, typed commands, the single-writer engine,
//! the journal, and project persistence. No GPU, no FFmpeg, no UI (docs/01 §4).

pub mod command;
pub mod effects;
pub mod engine;
pub mod error;
pub mod ids;
pub mod journal;
pub mod model;
pub mod runtime;
pub mod store;
pub mod time;

pub use command::{Command, Edge, Ripple, TimeInput};
pub use effects::{EffectCatalog, EffectManifest, ParamSpec, StaticCatalog};
pub use engine::{ApplyRequest, ApplyResult, Control, Diff, Engine, JournalEntry, Source};
pub use error::{CommandError, ErrorKind};
pub use model::Project;
pub use time::{FrameRate, Ratio, Round, Time};
