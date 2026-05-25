//! Provider adapters.

#![allow(async_fn_in_trait)]

use crate::capabilities::ProviderCapabilities;
use crate::types::ProviderRequest;
use crate::Result;

pub mod anthropic;
pub mod openai_compatible;

/// Trait implemented by all provider adapters.
pub trait ProviderAdapter: Send + Sync {
    /// Adapter name.
    fn name(&self) -> &str;
    /// Current capabilities snapshot.
    fn capabilities(&self) -> &ProviderCapabilities;
    /// Local token count (fast, no network).
    fn count_local(&self, request: &ProviderRequest) -> Result<u32>;
    /// Server-side token count (network I/O).
    async fn count_server(&self, request: &ProviderRequest) -> Result<u32>;
}
