//! hopscotch-core: everything in Hopscotch that actually touches the network.
//!
//! The desktop shell (src-tauri) is a thin layer over this crate, so all the
//! logic that can be unit-tested without a network lives here: parsing the
//! output of the system traceroute tools, deciding which addresses are
//! private, shaping RDAP and geolocation answers, and so on.

pub mod dns;
pub mod geo;
pub mod ipkind;
pub mod native;
pub mod netinfo;
pub mod parse;
pub mod ping;
pub mod process;
pub mod rdap;
pub mod trace;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("could not run {tool}: {source}")]
    Spawn { tool: String, source: std::io::Error },
    #[error("network request failed: {0}")]
    Http(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Http(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Message(format!("bad JSON: {e}"))
    }
}
