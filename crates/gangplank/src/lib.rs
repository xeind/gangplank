//! Hooks for GPUI: the mechanics every app rewrites, written once.
//!
//! Each hook is called during render, identified by its source location, and
//! hands back an entity the view reads. See each module for the contract.

mod debounce;
mod persisted;
mod resource;

pub use debounce::{DebouncedState, use_debounce, use_keyed_debounce};
pub use persisted::{Persisted, data_dir, use_keyed_persisted, use_persisted};
pub use resource::{Resource, ResourceState, use_keyed_resource, use_resource};
