//! `mimir-providers` — Provider gateway, capability registry, adapters, count,
//! retry, and redaction.
//!
//! This is the **only** crate that may speak HTTP to a provider.

#![warn(missing_docs)]

pub mod adapters;
pub mod capabilities;
pub mod count;
pub mod gateway;
pub mod retry;

pub use gateway::{ProviderGateway, ValidatedPacket};
