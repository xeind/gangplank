//! Hooks for GPUI: the mechanics every app rewrites, written once.
//!
//! Each hook is called during render, identified by its source location, and
//! hands back an entity the view reads. See each module for the contract.

#[cfg(all(feature = "ce", feature = "zed"))]
compile_error!("gangplank: enable one of the `ce` or `zed` features, not both");
#[cfg(not(any(feature = "ce", feature = "zed")))]
compile_error!("gangplank: enable the `ce` feature (gpui-ce) or `zed` (gpui from Zed)");

#[cfg(feature = "zed")]
extern crate gpui_zed as gpui;

mod clipboard;
mod debounce;
mod file_watch;
mod interval;
mod keyboard;
mod persisted;
mod previous;
mod resource;

pub use clipboard::{Clipboard, use_clipboard, use_keyed_clipboard};
pub use debounce::{DebouncedState, use_debounce, use_keyed_debounce};
pub use file_watch::{FileWatch, use_file_watch, use_keyed_file_watch};
pub use interval::{Interval, use_interval, use_keyed_interval};
pub use keyboard::{Shortcuts, use_keyboard, use_keyed_keyboard};
pub use persisted::{Persisted, data_dir, use_keyed_persisted, use_persisted};
pub use previous::{use_keyed_previous, use_previous};
pub use resource::{Resource, ResourceState, use_keyed_resource, use_resource};
