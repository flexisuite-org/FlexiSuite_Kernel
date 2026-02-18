pub mod key_manager;

// Re-export KeyManager
pub use key_manager::{KeyManager, KeyManagerError};

// Re-export types from kernel-data
pub use kernel_data::auth_context::{TenantContext, TenantId, UserId, SystemTenantContext, is_valid_principal};
