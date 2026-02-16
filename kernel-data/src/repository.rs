use async_trait::async_trait;
use kernel_core::kernel::Result;

/// Sealed trait to prevent external implementations.
mod private {
    pub trait Sealed {}
}

/// The public interface for tenant-scoped database operations.
/// This trait is sealed and can only be implemented within this crate.
#[async_trait]
pub trait TenantRepository: private::Sealed + Send + Sync {
    // Methods will be added here as we implement specific entity operations
}
