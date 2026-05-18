//! `mimir-providers` — Provider gateway, capability registry, adapters, count,
//! retry, and redaction.
//!
//! This is the **only** crate that may speak HTTP to a provider.

#![warn(missing_docs)]

pub mod adapters;
pub mod capabilities;
pub mod count;
pub mod error;
pub mod gateway;
pub mod retry;
pub mod types;

pub use error::{ProviderError, Result};
pub use gateway::{ProviderGateway, ValidatedPacket};
pub use types::{ProviderMessage, ProviderRequest, ProviderResponse, ResponseBlock, TokenUsage, ToolSchema};
