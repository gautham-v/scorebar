//! scorebar — live fantasy football matchups in the macOS menu bar.
//!
//! This file owns the plumbing only: the accessory activation policy, the
//! status item, the popover window (anchored under the item, toggled by a
//! click, closed on Esc, on an outside click or on focus loss, and resized to
//! its content), and the one detail window the popover opens on top of it. The
//! views live in `scorebar::ui`, the data in `scorebar::store_provider`.
//!
//! Two windows, and they are opposites. The popover is a `WindowKind::PopUp` —
//! a borderless, non-activating panel that behaves like a menu and dismisses
//! itself the moment attention moves. The detail window is an ordinary window
//! that has to stay put, which is the one thing an accessory app cannot get
//! for free: with no Dock icon scorebar is never the frontmost application, so
//! opening that window means activating the app as well as the window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::StreamExt;
use gpui::{
    point, px, size, App, AppContext, Application, Bounds, Entity, Focusable, Pixels, Subscription,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use objc2::MainThreadMarker;

use scorebar::status_item::{Anchor, MenuBarState, ScreenRect, StatusItem, StatusItemEvent};
use scorebar::store_provider::StoreProvider;
use scorebar::ui::popover::{self, Popover, PopoverEvent};
use scorebar::ui::provider::SnapshotProvider;
use scorebar::ui::theme;
use scorebar::ui::window::{self as detail_window, DetailWindow};
use sleeper::LeagueId;

/// Keep the popover this far from the screen edges.
const SCREEN_MARGIN: f32 = 8.0;
/// A status-item click that lands this soon after the popover closed itself is
/// the click that closed it; swallow it rather than reopening.
const TOGGLE_GRACE: std::time::Duration = std::time::Duration::from_millis(250);
/// The popover never gets shorter than this, however little room there is.
const MIN_POPOVER_HEIGHT: f32 = 160.0;
/// The popover window, when one is open.
type WindowSlot = Rc<RefCell<Option<WindowHandle<Popover>>>>;
/// The detail window, when one is open. There is only ever one: a manager in
/// four leagues wants tabs, not four windows.
type DetailSlot = Rc<RefCell<Option<WindowHandle<DetailWindow>>>>;
/// Where the open popover was placed, kept so a resize clamps the same way.
type PlacementSlot = Rc<Cell<Option<Placement>>>;

/// Where the popover goes: the left edge and top edge it is pinned to, and the
/// bottom of the display it is on.
///
/// Worked out once per open from the item's frame, and kept for as long as the
/// popover is up so that growing it — expanding Settings, a fetch that adds a
/// league — clamps against the same display the open did. Asking the window
/// where it thinks it is would not do: gpui reports a window's bounds relative
/// to whichever display it is on, which is not the space the open was
/// expressed in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    x: f32,
    top: f32,
    screen_bottom: f32,
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let mtm = MainThreadMarker::new().expect("gpui runs its callbacks on the main thread");
        scorebar::status_item::set_accessory_activation_policy(mtm);
        popover::bind_keys(cx);
        detail_window::bind_keys(cx);

        // Nothing has been fetched yet, so the item starts as a faded
        // scoreboard glyph with no score beside it.
        let (item, mut clicks) = StatusItem::new(mtm, MenuBarState::Idle);
        // Held for the life of the process; dropping it removes the menu bar item.
        let item = Rc::new(item);

        let store = Rc::new(StoreProvider::new(&cx.to_async()));
        let provider: Rc<dyn SnapshotProvider> = store.clone();

        // One popover entity for the whole run, re-rendered into whatever
        // window is open.
        let popover = cx.new({
            let provider = provider.clone();
            |cx| Popover::with_provider(provider, cx)
        });

        let window: WindowSlot = Rc::new(RefCell::new(None));
        let detail: DetailSlot = Rc::new(RefCell::new(None));

        // After any background fetch, or any settings change: re-title the
        // menu bar item, re-render the popover, and push the new week into the
        // detail window if one is open.
        {
            let item = item.clone();
            let popover = popover.clone();
            let detail = detail.clone();
            let store_for_hook = store.clone();
            store.set_on_change(Rc::new(move |cx: &mut App| {
                item.set_state(mtm, store_for_hook.menu_bar_state());
                popover.update(cx, |this, cx| this.reload(cx));
                push_to_detail(&detail, &store_for_hook, cx);
            }));
        }

        // First fill, then on the interval the settings ask for. The interval
        // is re-read every tick, so changing it in the Settings section takes
        // effect on the next one rather than on the next launch.
        store.refresh_if_stale();
        {
            let store = store.clone();
            cx.spawn(async move |cx| {
                let mut every = store.refresh_interval();
                loop {
                    cx.background_executor().timer(every).await;
                    let next = cx.update(|_| {
                        store.refresh_if_stale();
                        store.refresh_interval()
                    });
                    match next {
                        Ok(next) => every = next,
                        Err(_) => break, // app is shutting down
                    }
                }
            })
            .detach();
        }

        // When the popover closed itself on focus loss. A click on the status
        // item takes focus away first, so without this the close-then-click
        // ordering would read as "nothing was open" and reopen immediately.
        let closed_at: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));
        // Replaced on every open so the previous window's observer is dropped.
        let activation: Rc<RefCell<Option<Subscription>>> = Rc::new(RefCell::new(None));

        cx.subscribe(&popover, {
            let window = window.clone();
            let detail = detail.clone();
            let store = store.clone();
            move |_popover, event, cx| match event {
                PopoverEvent::Close => {
                    close_popover(&window, cx);
                }
                // The provider has already started the refetch; the change
                // hook re-renders when it lands.
                PopoverEvent::Refresh => {}
                // A league block was clicked. The popover is a menu, so it
                // gets out of the way before the window it summoned appears.
                PopoverEvent::OpenLeague(league_id) => {
                    close_popover(&window, cx);
                    open_detail(&detail, &store, league_id, cx);
                }
            }
        })
        .detach();

        // The popover grows and shrinks with the number of leagues and with
        // the Settings section, so keep the panel sized to the content.
        let placement: PlacementSlot = Rc::new(Cell::new(None));

        cx.observe(&popover, {
            let window = window.clone();
            let placement = placement.clone();
            move |popover, cx| {
                let Some(handle) = *window.borrow() else {
                    return;
                };
                let Some(placed) = placement.get() else {
                    return;
                };
                let height = popover.read(cx).preferred_height();
                // Growing a window keeps its top-left corner where it is, so
                // the popover stays under the item and only its bottom edge
                // moves.
                let height = clamp_height(height.into(), placed.top, placed.screen_bottom);
                let _ = handle.update(cx, |_, window, _| {
                    window.resize(size(theme::POPOVER_WIDTH, px(height)))
                });
            }
        })
        .detach();

        cx.spawn({
            let item = item.clone();
            let popover = popover.clone();
            let store = store.clone();
            let closed_at = closed_at.clone();
            let placement = placement.clone();
            async move |cx| {
                while let Some(event) = clicks.next().await {
                    if event == StatusItemEvent::ClickedOutside {
                        if cx.update(|cx| close_popover(&window, cx)).is_err() {
                            break;
                        }
                        continue;
                    }

                    let anchor = item.anchor(mtm);
                    let result = cx.update(|cx| {
                        // A click while the popover is up is a toggle.
                        if close_popover(&window, cx) {
                            closed_at.set(None);
                            return;
                        }
                        // The panel may have closed itself on focus loss a
                        // moment ago because of *this* click; don't reopen.
                        if closed_at.take().is_some_and(|t| t.elapsed() < TOGGLE_GRACE) {
                            return;
                        }
                        // Opening is also a refresh, but not a reason to spend
                        // a request: Sleeper's cdn holds these numbers for a
                        // minute, so an open inside that window reuses them.
                        store.refresh_if_stale();
                        popover.update(cx, |this, cx| this.reset(cx));
                        let height = popover.read(cx).preferred_height();
                        let placed = placement_for(cx, anchor);
                        placement.set(Some(placed));
                        match open_popover(
                            cx,
                            placed,
                            height,
                            popover.clone(),
                            &activation,
                            &window,
                            &closed_at,
                        ) {
                            Ok(handle) => *window.borrow_mut() = Some(handle),
                            Err(err) => eprintln!("scorebar: could not open popover: {err}"),
                        }
                    });
                    if result.is_err() {
                        break; // app is shutting down
                    }
                }
            }
        })
        .detach();
    });
}

/// Close the popover if one is open. Returns whether it closed something.
fn close_popover(window: &WindowSlot, cx: &mut App) -> bool {
    let handle = window.borrow_mut().take();
    match handle {
        // A stale handle (the window is already gone) updates with an error;
        // that counts as "nothing was open".
        Some(handle) => handle.update(cx, |_, w, _| w.remove_window()).is_ok(),
        None => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn open_popover(
    cx: &mut App,
    placed: Placement,
    height: Pixels,
    popover: Entity<Popover>,
    activation: &Rc<RefCell<Option<Subscription>>>,
    window_slot: &WindowSlot,
    closed_at: &Rc<Cell<Option<std::time::Instant>>>,
) -> anyhow::Result<WindowHandle<Popover>> {
    let bounds = popover_bounds(placed, height);

    let activation = activation.clone();
    let window_slot = window_slot.clone();
    let closed_at = closed_at.clone();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: true,
            show: true,
            // PopUp maps to a borderless, non-activating NSPanel on macOS,
            // which is exactly the menu-bar popover behavior.
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Blurred, with the popover painting a translucent background
            // (`theme::BG_ALPHA`) over it, is what gives the system-menu look:
            // whatever is behind the panel shows through as a soft wash.
            window_background: WindowBackgroundAppearance::Blurred,
            window_min_size: None,
            display_id: None,
            app_id: None,
            window_decorations: None,
            tabbing_identifier: None,
        },
        |window, cx| {
            let subscription = popover.update(cx, |_, cx| {
                // gpui fires this observer once at registration, before the
                // panel is key; only treat deactivation as a dismissal after
                // the window has genuinely been active.
                let was_active = Cell::new(false);
                cx.observe_window_activation(window, move |_, window, _| {
                    if window.is_window_active() {
                        was_active.set(true);
                    } else if was_active.get() {
                        // Clear the slot too, or the next status-item click
                        // finds a dead handle and reopens instead of toggling.
                        *window_slot.borrow_mut() = None;
                        closed_at.set(Some(std::time::Instant::now()));
                        window.remove_window();
                    }
                })
            });
            // Dropping the previous window's observer.
            *activation.borrow_mut() = Some(subscription);

            window.focus(&popover.focus_handle(cx));
            popover.clone()
        },
    )?;

    // The panel is non-activating, so ask AppKit to make it key explicitly.
    handle.update(cx, |_, window, _| window.activate_window())?;
    Ok(handle)
}

// ── The detail window ────────────────────────────────────────────────────────

/// Show `league_id` in the detail window: raise the one that is already open
/// and switch its tab, or open one.
///
/// Never two windows. Clicking a second league while the window is up is a
/// request to look at that league, not a request for another copy of the same
/// window, and [`DetailWindow::show_league`] is what makes the tab follow the
/// click.
///
/// The app is activated as well as the window because scorebar is an accessory
/// app: with no Dock icon it is never frontmost on its own, and a window
/// opened without activating lands behind whatever the user was doing.
fn open_detail(detail: &DetailSlot, store: &Rc<StoreProvider>, league_id: &LeagueId, cx: &mut App) {
    // Copied out first: the slot is written below, and holding a borrow across
    // that would panic.
    let open = *detail.borrow();
    if let Some(handle) = open {
        let leagues = store.details();
        let raised = handle.update(cx, |view, window, cx| {
            view.set_leagues(leagues, cx);
            view.show_league(league_id, cx);
            window.activate_window();
        });
        if raised.is_ok() {
            cx.activate(true);
            return;
        }
        // The window was closed from its own traffic light or with Esc, which
        // leaves a handle that no longer resolves. Drop it and open a new one.
        *detail.borrow_mut() = None;
    }

    match detail_window::open(cx, store.details(), Some(league_id)) {
        Ok(handle) => {
            *detail.borrow_mut() = Some(handle);
            cx.activate(true);
        }
        Err(err) => eprintln!("scorebar: could not open the detail window: {err}"),
    }
}

/// Push the week that has just landed into the open detail window, if there is
/// one. A dead handle means the user closed it; forget it rather than trying
/// again every minute.
fn push_to_detail(detail: &DetailSlot, store: &Rc<StoreProvider>, cx: &mut App) {
    let open = *detail.borrow();
    let Some(handle) = open else {
        return;
    };
    let leagues = store.details();
    if handle
        .update(cx, |view, _, cx| view.set_leagues(leagues, cx))
        .is_err()
    {
        *detail.borrow_mut() = None;
    }
}

// ── Placing the popover ──────────────────────────────────────────────────────

/// Work out where the popover goes: centred under the status item, its top
/// edge under the menu bar, and kept inside the display the item is on rather
/// than inside the primary one.
fn placement_for(cx: &App, anchor: Option<Anchor>) -> Placement {
    let b = display_bounds(cx);
    let primary = ScreenRect {
        x: b.origin.x.into(),
        y: b.origin.y.into(),
        width: b.size.width.into(),
        height: b.size.height.into(),
    };
    placement(anchor, primary)
}

/// The placement itself, over the item's anchor and — for the case where
/// AppKit would not say where the item is — the primary display to fall back
/// to. Pure, so the arrangement-dependent arithmetic is testable.
fn placement(anchor: Option<Anchor>, primary: ScreenRect) -> Placement {
    let width: f32 = theme::POPOVER_WIDTH.into();
    let gap: f32 = theme::POPOVER_TOP_GAP.into();

    let screen = anchor.map_or(primary, |a| a.screen);
    let left = screen.x + SCREEN_MARGIN;
    let right = (screen.x + screen.width - width - SCREEN_MARGIN).max(left);

    let (x, top) = match anchor {
        Some(a) => {
            let centered = a.item.x + a.item.width / 2.0 - width / 2.0;
            (centered.clamp(left, right), a.item.y + a.item.height + gap)
        }
        // No status item frame (shouldn't happen): hug the primary display's
        // top-right corner.
        None => (right, screen.y + 28.0 + gap),
    };

    Placement {
        x,
        top,
        screen_bottom: screen.y + screen.height,
    }
}

/// The window bounds for a placement and a content height.
fn popover_bounds(placed: Placement, height: Pixels) -> Bounds<Pixels> {
    let height = clamp_height(height.into(), placed.top, placed.screen_bottom);
    Bounds {
        origin: point(px(placed.x), px(placed.top)),
        size: size(theme::POPOVER_WIDTH, px(height)),
    }
}

fn display_bounds(cx: &App) -> Bounds<Pixels> {
    cx.primary_display().map(|d| d.bounds()).unwrap_or(Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(1440.), px(900.)),
    })
}

/// A popover with a dozen leagues in it can want more height than a short
/// screen has. Whatever does not fit is clipped rather than growing off the
/// bottom edge; the popover itself never scrolls.
fn clamp_height(height: f32, top_y: f32, display_bottom: f32) -> f32 {
    let room = display_bottom - top_y - SCREEN_MARGIN;
    height.min(room.max(MIN_POPOVER_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f32, y: f32, width: f32, height: f32) -> ScreenRect {
        ScreenRect {
            x,
            y,
            width,
            height,
        }
    }

    /// The built-in display as the primary one, with a 2560-wide monitor
    /// arranged above and to the left of it — the arrangement that shows up
    /// the clamping bug, since the monitor's right edge is well past the
    /// primary display's width.
    fn second_screen() -> ScreenRect {
        screen(-561.0, -1440.0, 2560.0, 1440.0)
    }

    fn anchor_on(screen: ScreenRect, x: f32) -> Option<Anchor> {
        Some(Anchor {
            item: ScreenRect {
                x,
                y: screen.y,
                width: 60.0,
                height: 24.0,
            },
            screen,
        })
    }

    #[test]
    fn anchor_math_centers_under_the_item() {
        let width: f32 = theme::POPOVER_WIDTH.into();
        let primary = screen(0.0, 0.0, 1440.0, 900.0);
        let placed = placement(anchor_on(primary, 1000.0), primary);
        assert_eq!(placed.x, 1000.0 + 30.0 - width / 2.0);
        assert_eq!(placed.top, 24.0);
    }

    #[test]
    fn anchor_math_clamps_to_the_right_edge() {
        let width: f32 = theme::POPOVER_WIDTH.into();
        let primary = screen(0.0, 0.0, 1440.0, 900.0);
        let placed = placement(anchor_on(primary, 1400.0), primary);
        assert_eq!(placed.x, 1440.0 - width - SCREEN_MARGIN);
    }

    /// The bug the popover's alignment comes down to: an item on a second
    /// display is clamped against *that* display, not against the primary
    /// one, or it lands hundreds of points to the left of the item it belongs
    /// to.
    #[test]
    fn an_item_on_a_second_display_stays_under_the_item() {
        let width: f32 = theme::POPOVER_WIDTH.into();
        let second = second_screen();
        let placed = placement(anchor_on(second, 1600.0), screen(0.0, 0.0, 1512.0, 982.0));
        assert_eq!(placed.x, 1600.0 + 30.0 - width / 2.0);
        // And the room under it is that display's, so a tall popover is not
        // cut off at the primary display's bottom edge.
        assert_eq!(placed.top, second.y + 24.0);
        assert_eq!(placed.screen_bottom, 0.0);
    }

    #[test]
    fn a_tall_popover_is_clamped_to_the_screen() {
        assert_eq!(clamp_height(2070.0, 32.0, 900.0), 900.0 - 32.0 - 8.0);
        // A short popover is left alone.
        assert_eq!(clamp_height(300.0, 32.0, 900.0), 300.0);
        // Never collapse to nothing on a tiny screen.
        assert_eq!(clamp_height(800.0, 890.0, 900.0), MIN_POPOVER_HEIGHT);
    }

    #[test]
    fn a_resize_keeps_the_top_edge_and_only_grows_downwards() {
        let placed = placement(
            anchor_on(screen(0.0, 0.0, 1440.0, 900.0), 1000.0),
            screen(0.0, 0.0, 1440.0, 900.0),
        );
        let short = popover_bounds(placed, px(300.0));
        let tall = popover_bounds(placed, px(500.0));
        assert_eq!(short.origin, tall.origin);
        assert_eq!(tall.size.height, px(500.0));
    }
}
