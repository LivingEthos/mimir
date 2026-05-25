//! `mimir-schemas` — JSON Schema types and validation.
//!
//! This crate owns the generated Rust types from the JSON Schemas under
//! `schemas/`. It is the bottom of the dependency stack: every other crate
//! depends on it, and it depends on nothing in the workspace.

#![warn(missing_docs)]

pub mod generated;

pub use generated::*;
