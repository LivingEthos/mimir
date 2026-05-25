//! `mimir-providers` — Provider gateway, capability registry, adapters, count,
//! retry, and redaction.
//!
//! This is the **only** crate that may speak HTTP to a provider.

#![warn(missing_docs)]

pub mod adapters;
pub mod cache;
pub mod capabilities;
pub mod count;
pub mod error;
pub mod gateway;
pub mod retry;
pub mod stream;
pub mod types;

pub use adapters::openai_compatible::{OpenAiCompatibleAdapter, OpenAiCompatibleConfig};
pub use adapters::ProviderAdapter;
pub use error::{ProviderError, Result};
pub use gateway::{
    ProviderDispatchAdapter, ProviderGateway, ValidatedPacket, ValidatedProviderRequest,
};
pub use types::{
    ProviderMessage, ProviderRequest, ProviderResponse, ResponseBlock, TokenUsage, ToolSchema,
};
