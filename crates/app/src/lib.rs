//! scorebar as a library, so examples and tests can build the views without
//! going through the menu bar binary.
//!
//! Three layers, and only the last one needs a Mac in front of it.
//!
//! - **On disk.** [`settings`] is the hand-editable `config.toml`; [`cache`]
//!   is the last good week and the player dictionary, neither of which is
//!   ever the source of truth.
//! - **In the menu bar.** [`status_item`] is the `NSStatusItem` and the bridge
//!   from an AppKit click into gpui, [`menu_bar_icon`] is the picture it
//!   draws.
//! - **The app.** [`store_provider`] is the refresh loop and the seam the
//!   views are written against; [`ui`] is the popover and the detail window.
//!
//! The binary — `main.rs` — holds none of that. It is the window plumbing and
//! nothing else, which is what keeps everything here buildable by
//! `examples/popover_preview.rs` and assertable by `cargo test`.

pub mod cache;
pub mod menu_bar_icon;
pub mod settings;
pub mod status_item;
pub mod store_provider;
pub mod ui;
