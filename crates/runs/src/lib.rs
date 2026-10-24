//! Research view parsing: bytes in, normalized runs out (docs/research-view.md).
//!
//! This crate takes text and returns values. It has no file access, no async runtime and no
//! dependency on `proto`, `client`, `host` or GPUI, so every behavior is testable from
//! fixtures. It reads two formats, verifiers v0.4.0 `traces.jsonl` and Harbor 0.24.0 job and
//! trial directories, into the one model in [`model`].
//!
//! The model types are also the wire types of the research messages in `seamlessh-proto`
//! (postcard). They therefore avoid serde attributes postcard cannot round-trip, and a field
//! or variant is only ever appended, never reordered.

pub mod harbor;
pub mod matrix;
pub mod model;
mod time;
pub mod verifiers;

pub use model::*;
