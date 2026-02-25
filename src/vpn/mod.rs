//! VPN proxy support for ZeroClaw.
//!
//! This module provides VPN proxy functionality using an external Clash
//! process as the proxy runtime and subconverter for Clash subscription parsing.
//! Enabled via `--features vpn`.

pub mod bridge;
pub mod bypass;
pub mod health;
pub mod node_manager;
pub mod runtime;
pub mod subscription;

pub use bridge::VpnProxyBridge;
pub use bypass::{BypassChecker, BypassDecision};
pub use health::{HealthChecker, HealthResult, NodeStatus};
pub use node_manager::{NodeCache, NodeManager};
pub use runtime::{generate_clash_config, ClashRuntime};
pub(crate) use runtime::{CLASH_CONTROLLER_PORT, SELECTOR_GROUP_NAME};
pub use subscription::{NodeType, ProxyNode, SubscriptionParser};
