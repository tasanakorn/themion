//! Shared types, constants, and errors for stylos.

use thiserror::Error;

pub const STYLOS_MULTICAST_ADDR: &str = "224.0.0.224:31746";
pub const STYLOS_DEFAULT_DATA_PORT: u16 = 31747;
pub const STYLOS_PORT_WALK_CAP: u16 = 8;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Error, Debug)]
pub enum StylosError {
    #[error("config error: {0}")]
    Config(String),
    #[error("identity error: {0}")]
    Identity(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("session error: {0}")]
    Session(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON5 parse error: {0}")]
    Json5(String),
    #[error("zenoh error: {0}")]
    Zenoh(String),
}

impl From<json5::Error> for StylosError {
    fn from(e: json5::Error) -> Self { StylosError::Json5(e.to_string()) }
}

pub type Result<T> = std::result::Result<T, StylosError>;
