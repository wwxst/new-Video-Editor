//! Opaque, prefixed, sortable entity IDs (docs/02 §1).
//!
//! Wire form: `<prefix>_<chars>`, e.g. `cl_01J8ZK…` or a client-chosen `tr_music`.
//! Clients treat IDs as equality-only.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid id {value:?}: expected prefix {prefix:?} followed by 1-64 of [A-Za-z0-9_]")]
pub struct IdError {
    pub prefix: &'static str,
    pub value: String,
}

fn valid(prefix: &str, s: &str) -> bool {
    match s.strip_prefix(prefix).and_then(|r| r.strip_prefix('_')) {
        Some(rest) => {
            !rest.is_empty() && rest.len() <= 64 && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }
        None => false,
    }
}

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Generate a fresh, time-ordered ID.
            pub fn generate() -> Self {
                Self(format!("{}_{}", $prefix, ulid::Ulid::new()))
            }

            pub fn parse(s: &str) -> Result<Self, IdError> {
                if valid($prefix, s) {
                    Ok(Self(s.to_string()))
                } else {
                    Err(IdError { prefix: $prefix, value: s.to_string() })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

id_type!(ProjectId, "prj");
id_type!(SequenceId, "sq");
id_type!(TrackId, "tr");
id_type!(ClipId, "cl");
id_type!(AssetId, "as");
id_type!(EffectId, "fx");
id_type!(TransitionId, "tx");
id_type!(MarkerId, "mk");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_generate() {
        assert!(ClipId::parse("cl_abc").is_ok());
        assert!(ClipId::parse("tr_abc").is_err());
        assert!(ClipId::parse("cl_").is_err());
        assert!(ClipId::parse("cl_a-b").is_err());
        let a = ClipId::generate();
        let b = ClipId::generate();
        assert_ne!(a, b);
        assert!(ClipId::parse(a.as_str()).is_ok());
        let j = serde_json::to_string(&a).expect("ser");
        let back: ClipId = serde_json::from_str(&j).expect("de");
        assert_eq!(a, back);
    }
}
