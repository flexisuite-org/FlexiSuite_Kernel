use std::fmt;
use std::str::FromStr;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

macro_rules! define_principal_id {
    ($type_name:ident, $label:literal) => {
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
                if is_valid_principal(&s) {
                    Ok(Self(s))
                } else {
                    Err(format!("Invalid {} format: {}", $label, s))
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

define_principal_id!(TenantId, "tenant_id");
define_principal_id!(UserId, "user_id");

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
