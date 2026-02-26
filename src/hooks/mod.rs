pub mod audit;
pub mod builtin;
pub mod cli;
pub mod dynamic;
pub mod loader;
pub mod manifest;
pub mod reload;
pub use dynamic::*;
pub use loader::*;
pub use manifest::*;
mod runner;
mod traits;

pub use runner::HookRunner;
// HookHandler and HookResult are part of the crate's public hook API surface.
// They may appear unused internally but are intentionally re-exported for
// external integrations and future plugin authors.
#[allow(unused_imports)]
pub use traits::{HookHandler, HookResult};
#[cfg(test)]
mod integration_tests;
