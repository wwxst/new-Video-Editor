//! Typed command errors with stable `kind` strings (docs/06 §4.1).

use crate::time::TimeError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidCommand,
    StaleRevision,
    EmptyBatch,
    BatchTooLarge,
    ForbiddenOp,
    NotInControl,
    OffFrameBoundary,
    InvalidTimecode,
    DropFrameNotAllowed,
    UnsupportedFrameRate,
    InexactFrameRate,
    TimeOverflow,
    InvalidId,
    DuplicateId,
    UnknownTrack,
    UnknownClip,
    UnknownAsset,
    UnknownEffect,
    UnknownTransition,
    UnknownMarker,
    TrackKindMismatch,
    TrackLocked,
    Overlap,
    OutOfRange,
    InvalidValue,
    SourceRangeExceeded,
    AssetInUse,
    TransitionInvalid,
    TransitionBroken,
    UnsupportedRipple,
    NothingToUndo,
    NothingToRedo,
    JournalUnavailable,
}

impl ErrorKind {
    pub fn as_str(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
    /// JSON path of the offending input, e.g. `commands[2].start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        CommandError { kind, message: message.into(), path: None, details: None }
    }
    pub fn at(mut self, path: impl Into<String>) -> Self {
        if self.path.is_none() {
            self.path = Some(path.into());
        }
        self
    }
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl From<TimeError> for CommandError {
    fn from(e: TimeError) -> Self {
        let kind = match e {
            TimeError::Overflow | TimeError::ZeroDenominator => ErrorKind::TimeOverflow,
            TimeError::OffFrameBoundary(_) => ErrorKind::OffFrameBoundary,
            TimeError::InvalidTimecode(_) => ErrorKind::InvalidTimecode,
            TimeError::DropFrameNotAllowed => ErrorKind::DropFrameNotAllowed,
            TimeError::UnsupportedFrameRate(_) => ErrorKind::UnsupportedFrameRate,
            TimeError::InexactFrameRate(_) => ErrorKind::InexactFrameRate,
            TimeError::Negative => ErrorKind::OutOfRange,
        };
        CommandError::new(kind, e.to_string())
    }
}

pub type CmdResult<T> = Result<T, CommandError>;
