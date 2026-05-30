//! Control plane REST handlers split out from main.rs.
//!
//! SLICE_07 ships the first such split — `predictor_plugins` — because
//! the plugin endpoint registry is conceptually distinct from the
//! tenant/budget provisioning surface that main.rs hosts. Future
//! slices may move the existing handlers here as well.

pub mod predictor_plugins;
