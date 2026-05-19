//! Panel implementations for the Mimir TUI.

pub mod budget;
pub mod diff;
pub mod included;
pub mod omitted;
pub mod permissions;
pub mod provider_count;

pub use budget::BudgetPanel;
pub use diff::DiffPanel;
pub use included::IncludedPanel;
pub use omitted::OmittedPanel;
pub use permissions::PermissionsPanel;
pub use provider_count::ProviderCountPanel;
