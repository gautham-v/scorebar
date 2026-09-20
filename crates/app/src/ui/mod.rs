//! The two gpui views, and what they are both drawn from.
//!
//! [`popover`] is the menu under the status item: 260px, a block per league,
//! no lineups. [`window`] is the detail window it opens — an ordinary titled
//! window with room for the starting lineups, tabbed by league.
//!
//! Neither talks to the network or to `scorebar-core` directly: they hold a
//! [`provider`] and ask it for the week. That is what keeps the whole view
//! tree renderable — and testable — with no username and no network, which is
//! what `examples/popover_preview.rs` runs on. All colors and sizes come from
//! [`theme`] — no literals in the views.

pub mod popover;
pub mod provider;
pub mod theme;
pub mod window;
