use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

macro_rules! define_principal_id {
    ($type_name:ident, $label:literal, $reserve_system:expr) => {
        #[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $type_name(String);

        impl<'de> Deserialize<'de> for $type_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct PrincipalVisitor;

                impl<'de> Visitor<'de> for PrincipalVisitor {
                    type Value = $type_name;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str(concat!("a valid ", $label, " string"))
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        $type_name::new(value).map_err(de::Error::custom)
                    }
                }

                deserializer.deserialize_str(PrincipalVisitor)
            }
        }

        impl $type_name {
            pub fn new(id: impl Into<String>) -> Result<Self, String> {
                let s = id.into();
                if is_valid_principal(&s) && !($reserve_system && s == "system") {
                    Ok(Self(s))
                } else {
                    Err(format!("Invalid {} format", $label))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub(crate) fn new_unchecked(id: impl Into<String>) -> Self {
                Self(id.into())
            }
        }

        impl fmt::Display for $type_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl AsRef<str> for $type_name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $type_name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }
    };
}

define_principal_id!(TenantId, "tenant_id", true);
define_principal_id!(UserId, "user_id", false);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TenantContext {
    tenant_id: TenantId,
    user_id: Option<UserId>,
    #[serde(skip)]
    db: Option<std::sync::Arc<sea_orm::DatabaseConnection>>,
}

impl TenantContext {
    pub fn new(tenant_id: TenantId, user_id: Option<UserId>) -> Self {
        Self {
            tenant_id,
            user_id,
            db: None,
        }
    }

    pub fn with_db(mut self, db: std::sync::Arc<sea_orm::DatabaseConnection>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }

    pub fn is_system(&self) -> bool {
        self.tenant_id.as_str() == "system"
    }

    pub fn db(&self) -> Result<&sea_orm::DatabaseConnection, String> {
        self.db
            .as_deref()
            .ok_or_else(|| "Database connection not attached to context".to_string())
    }
}

/// A marker type for operations that are inherently system-wide and not
/// bound to a specific end-user tenant.
///
/// This exists to satisfy the requirement that all database-facing APIs
/// accept a context, even for bootstrap or maintenance tasks.
pub struct SystemTenantContext;

impl From<SystemTenantContext> for TenantContext {
    fn from(_: SystemTenantContext) -> Self {
        Self {
            tenant_id: TenantId::new_unchecked("system"),
            user_id: None,
            db: None,
        }
    }
}

pub fn is_valid_principal(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes
            .iter()
            .all(|b| matches!(*b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_rejects_reserved_system() {
        assert!(TenantId::new("system").is_err());
    }

    #[test]
    fn system_context_still_uses_reserved_id() {
        let ctx = TenantContext::from(SystemTenantContext);
        assert!(ctx.is_system());
        assert_eq!(ctx.tenant_id().as_str(), "system");
    }

    #[test]
    fn user_id_allows_system_literal() {
        assert!(UserId::new("system").is_ok());
    }
}
