//! Stylos identity: Realm/Role/Instance newtypes + key-expr composer.

use serde::{Deserialize, Serialize};
use stylos_common::{Result, StylosError};

fn validate_segment(s: &str, field: &str) -> Result<()> {
    if s.is_empty() {
        return Err(StylosError::Identity(format!("{field} must not be empty")));
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err(StylosError::Identity(format!(
            "{field} must start with lowercase letter or digit: {s}"
        )));
    }
    for b in bytes {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
        if !ok {
            return Err(StylosError::Identity(format!(
                "{field} must match [a-z0-9][a-z0-9-]*: {s}"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Realm(String);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role(String);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance(String);

impl Realm    { pub fn new(s: impl Into<String>) -> Result<Self> { let s = s.into(); validate_segment(&s, "realm")?;    Ok(Self(s)) } pub fn as_str(&self) -> &str { &self.0 } }
impl Role     { pub fn new(s: impl Into<String>) -> Result<Self> { let s = s.into(); validate_segment(&s, "role")?;     Ok(Self(s)) } pub fn as_str(&self) -> &str { &self.0 } }
impl Instance { pub fn new(s: impl Into<String>) -> Result<Self> { let s = s.into(); validate_segment(&s, "instance")?; Ok(Self(s)) } pub fn as_str(&self) -> &str { &self.0 } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StylosIdentity {
    pub realm: Realm,
    pub role: Role,
    pub instance: Instance,
}

impl StylosIdentity {
    pub fn new(realm: &str, role: &str, instance: &str) -> Result<Self> {
        Ok(Self {
            realm: Realm::new(realm)?,
            role: Role::new(role)?,
            instance: Instance::new(instance)?,
        })
    }

    pub fn root_key(&self) -> String {
        format!("stylos/{}/{}/{}", self.realm.as_str(), self.role.as_str(), self.instance.as_str())
    }
}
