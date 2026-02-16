use std::fmt;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantId(String);

impl<'de> Deserialize<'de> for TenantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TenantIdVisitor;

        impl<'de> Visitor<'de> for TenantIdVisitor {
            type Value = TenantId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid tenant_id string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                TenantId::new(value).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(TenantIdVisitor)
    }
}

impl TenantId {
    pub fn new(id: impl Into<String>) -> Result<Self, String> {
        let s = id.into();
        if is_valid_principal(&s) {
            Ok(Self(s))
        } else {
            Err(format!("Invalid tenant_id format: {}", s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(feature = "test-utils")]
    pub fn new_unchecked(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(String);

impl<'de> Deserialize<'de> for UserId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UserIdVisitor;

        impl<'de> Visitor<'de> for UserIdVisitor {
            type Value = UserId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid user_id string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                UserId::new(value).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(UserIdVisitor)
    }
}

impl UserId {
    pub fn new(id: impl Into<String>) -> Result<Self, String> {
        let s = id.into();
        if is_valid_principal(&s) {
            Ok(Self(s))
        } else {
            Err(format!("Invalid user_id format: {}", s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TenantContext {
    tenant_id: TenantId,
    user_id: Option<UserId>,
}

impl TenantContext {
    pub fn new(tenant_id: TenantId, user_id: Option<UserId>) -> Self {
        Self { tenant_id, user_id }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }
}

pub fn is_valid_principal(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes.iter().all(
            |b| matches!(*b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'),
        )
}
