pub mod config;
pub mod error;
pub mod paths;

pub use config::{Config, PromotionConfig};
pub use error::{Error, Result};
pub use paths::Paths;
