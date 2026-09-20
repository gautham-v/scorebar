//! Root popover view: the week header, a block per league, and the menu rows
//! under them.
//!
//! It is shaped like a system menu — the Battery item's menu: 260px wide, 5px
//! of inset around plain rows, a semibold header, hairline separators, and
//! nothing but ink on the material. There is no accent colour anywhere,
//! because the one thing this popover has to say — who is winning — is said
//! with ink against grey (see [`Theme::score_ink`]) and with how far a meter
//! has filled. A scoreboard that painted the leader green would lose that
//! reader, and would still need the contrast to be legible.
//!
//! A league block is three lines: the name and the two scores, a meter under
//! them filled by *my* win probability, and the two projected finals. A short
//! bar is a losing bar, which is why the meter is drawn from my side rather
//! than as a share of the points on the board.
//!
//! It owns the little state there is — the theme picked from the window's
//! appearance and whether the Settings row is expanded — plus the key
//! bindings. Everything it knows about the week arrives through
//! [`SnapshotProvider`], so the whole view tree renders against
//! [`StubProvider`](crate::ui::provider::StubProvider) with no username and no
//! network — see `examples/popover_preview.rs`.
//!
//! The three states that are not the happy one matter as much as it does:
//!
//! - **No username.** One line inviting one, and the Settings section already
//!   open under it, so the fix is where the problem is stated.
//! - **A failed fetch.** The last good numbers stay on screen one shade back,
//!   with an 11px line saying what failed and when the numbers are from. The
//!   popover never goes blank over a dropped connection.
//! - **No game this week.** A league with nobody on the other side collapses
//!   to a single line saying so, rather than drawing a matchup out of zeros.

use std::rc::Rc;

use chrono::{DateTime, Datelike, Local, Weekday};
use gpui::prelude::FluentBuilder;
use gpui::{
    actions, div, px, relative, App, Context, Div, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Render, Rgba,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use scorebar_core::{LeagueCard, Side, Snapshot};
use sleeper::LeagueId;

use crate::settings::{MenuBarTitle, Settings, MIN_REFRESH_SECONDS};
use crate::ui::provider::SnapshotProvider;
use crate::ui::theme::{self, Theme};

/// Where the "Open Sleeper" row goes: the league list, not one league, because
/// the popover cannot know which league the click was about.
pub const SLEEPER_URL: &str = "https://sleeper.com/leagues";

/// What the popover asks its window owner to do. The popover can redraw
/// itself, and nothing else: closing the panel, refetching and opening a
/// second window all belong to whatever owns it.
pub enum PopoverEvent {
    /// Esc, or any other dismissal — close the window.
    Close,
    /// Refresh was asked for — the owner refetches and redraws the menu bar
    /// item, which the popover itself cannot reach.
    Refresh,
    /// A league block was clicked — open the detail window on that league.
    OpenLeague(LeagueId),
}

actions!(scorebar, [Refresh, Dismiss]);

/// Key context for the popover. There is only one screen, so there is only one.
pub const KEY_CONTEXT: &str = "Scorebar";

/// Install the popover's key bindings. Call once at app start.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
        KeyBinding::new("r", Refresh, Some(KEY_CONTEXT)),
    ]);
}

pub struct Popover {
    focus: FocusHandle,
    provider: Rc<dyn SnapshotProvider>,
    /// Whether the "Settings" disclosure row has its rows shown under it.
    settings_open: bool,
    theme: Theme,
    appearance: Option<gpui::Subscription>,
}

impl Popover {
    /// A popover over the fixture provider — what the preview example uses,
    /// and what the binary falls back to before the real layers are wired up.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_provider(Rc::new(crate::ui::provider::StubProvider::new()), cx)
    }

    /// Build a popover over any provider.
    pub fn with_provider(provider: Rc<dyn SnapshotProvider>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            provider,
            settings_open: false,
            theme: Theme::default(),
            appearance: None,
        }
    }

    // ── What the leaves read ────────────────────────────────────────────────

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn provider(&self) -> &Rc<dyn SnapshotProvider> {
        &self.provider
    }

    /// The clock the header and the staleness line read.
    pub fn now(&self) -> DateTime<Local> {
        Local::now()
    }

    /// The week on screen, if one has ever landed.
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.provider.snapshot()
    }

    /// Whether there is a username to fetch with. Without one the popover has
    /// nothing to say and says so in one line.
    pub fn is_configured(&self) -> bool {
        self.provider.settings().is_configured()
    }

    /// Whether the numbers on screen are from a fetch that has since failed.
    /// They stay up, one shade back, under a line saying when they are from.
    fn is_stale(&self) -> bool {
        self.provider.error().is_some() && self.provider.snapshot().is_some()
    }

    /// The cards to draw, which is nothing at all until a fetch lands.
    fn leagues(&self) -> Vec<LeagueCard> {
        self.provider
            .snapshot()
            .map(|snapshot| snapshot.leagues)
            .unwrap_or_default()
    }

    // ── State changes ───────────────────────────────────────────────────────

    /// Back to the default state; called every time the popover opens.
    ///
    /// With no username the Settings section starts open: the popover's only
    /// line is an invitation to add one, and leaving the fix one click away
    /// behind a row labelled "Settings" is a worse first launch than showing
    /// it outright.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.is_configured();
        self.reload(cx);
    }

    /// Redraw against whatever the provider holds now. The provider owns the
    /// snapshot, so there is nothing to copy — this is the hook the owner
    /// calls when a fetch lands.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Ask for a refetch and tell the owner, which also redraws the menu bar.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.provider.refresh();
        cx.emit(PopoverEvent::Refresh);
        self.reload(cx);
    }

    /// Expand the Settings section without a click — the preview example's
    /// `settings` mode, which cannot click.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    fn open_sleeper(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.open_url(SLEEPER_URL);
    }

    /// Hand the league up to the owner, which opens the detail window on it.
    fn open_league(&mut self, league_id: LeagueId, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.emit(PopoverEvent::OpenLeague(league_id));
    }

    /// Open `~/.config/scorebar/config.toml` in whatever edits text here.
    ///
    /// This is how the username is set. The popover has no text field: gpui
    /// gives a view key events and a caret and nothing else, and a hand-rolled
    /// one-line editor inside a panel that closes when it loses focus is a
    /// worse way to type eight characters than the file the setting already
    /// lives in.
    fn open_config_file(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = config_url() {
            cx.open_url(&url);
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        // Esc collapses the Settings section first, so one press never does
        // two things.
        if self.settings_open {
            self.settings_open = false;
            cx.notify();
        } else {
            cx.emit(PopoverEvent::Close);
        }
    }

    // ── Layout ──────────────────────────────────────────────────────────────

    /// Height the content wants; the window is resized to it.
    pub fn preferred_height(&self) -> Pixels {
        let notice = if self.notice().is_some() {
            NOTICE_HEIGHT
        } else {
            0.0
        };
        px(POPOVER_PAD_TOTAL
            + SECTION_HEADER_HEIGHT
            + notice
            + self.leagues_height()
            + SEPARATOR_HEIGHT
            + self.menu_height())
    }

    /// How tall the league list is: one block per league — two heights,
    /// depending on whether there is a game — or the one line that stands in
    /// for them when there are none.
    fn leagues_height(&self) -> f32 {
        let leagues = self.leagues();
        if leagues.is_empty() {
            return LEAGUES_PAD_TOP + theme::LINE_TITLE_PX + LEAGUES_PAD_BOTTOM;
        }
        let blocks: f32 = leagues.iter().map(block_height).sum();
        let gaps = (leagues.len() as f32 - 1.0) * theme::BLOCK_GAP_PX;
        LEAGUES_PAD_TOP + blocks + gaps + LEAGUES_PAD_BOTTOM
    }

    /// How tall the menu rows at the bottom are, from the rows they will draw.
    /// Kept as arithmetic over the row constants rather than measured, because
    /// `preferred_height` runs before the rows are laid out.
    fn menu_height(&self) -> f32 {
        // Refresh, Open Sleeper and Settings, then Quit under its own
        // separator.
        let rows = 4.0;
        let settings = if self.settings_open {
            SETTINGS_HEIGHT
        } else {
            0.0
        };
        rows * ROW_HEIGHT + settings + SEPARATOR_HEIGHT
    }

    // ── The header and the league blocks ────────────────────────────────────

    /// "Week 2" on the left, the day of the week on the right. The day is the
    /// one piece of context a fantasy score needs and cannot carry: the same
    /// 56.92 means two different things on Sunday lunchtime and on Tuesday.
    fn header(&self) -> impl IntoElement {
        let theme = self.theme;
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_baseline()
            .px(theme::ROW_PAD_X)
            .pt(px(SECTION_HEADER_PAD_TOP))
            .pb(px(SECTION_HEADER_PAD_BOTTOM))
            .text_size(theme::TEXT_TITLE)
            .line_height(theme::LINE_TITLE)
            .child(
                div()
                    .font_weight(theme::WEIGHT_EMPHASIS)
                    .child(week_label(self.snapshot().map(|s| s.week))),
            )
            .child(
                div()
                    .text_color(theme.secondary)
                    .child(day_label(self.now().weekday())),
            )
    }

    /// The muted 11px line under the header, when there is something to say.
    /// A healthy popover has nothing there — the blocks already say it.
    fn notice(&self) -> Option<String> {
        notice_text(
            self.provider.error().as_deref(),
            self.provider.snapshot().map(|s| s.fetched_at),
            self.now(),
        )
    }

    fn notice_line(&self) -> Option<impl IntoElement> {
        let theme = self.theme;
        self.notice().map(|line| {
            div()
                .px(theme::ROW_PAD_X)
                .h(px(NOTICE_HEIGHT))
                .text_size(theme::TEXT_TINY)
                .line_height(theme::LINE_TINY)
                .text_color(theme.secondary)
                .truncate()
                .child(line)
        })
    }

    /// One league: its name and the two scores, the meter, and the two
    /// projected finals. The whole block washes on hover and the whole block
    /// is the click target, because a three line block with one clickable line
    /// in it is a worse target than a block that means what it looks like.
    fn league_block(&self, card: &LeagueCard, cx: &mut Context<Self>) -> Div {
        let ink = Ink::new(self.theme, self.is_stale());
        let mut block = self.block_frame(card, cx).child(self.score_line(card, ink));
        // The meter and the projections only exist when there is a game on;
        // without one the block is the single line `score_line` drew.
        if let Some(opponent) = card.opponent.as_ref() {
            block = block.child(self.meter_line(card.win_percent(), ink)).child(
                div()
                    .text_size(theme::TEXT_TINY)
                    .line_height(theme::LINE_TINY)
                    .text_color(ink.tertiary)
                    .truncate()
                    .child(projection_text(&card.me, opponent)),
            );
        }
        // The blocks live in a flex column and gpui's `Stateful<Div>` is not a
        // `Div`; wrapping keeps every child of the column the same type.
        div().flex().flex_col().child(block)
    }

    /// The clickable frame a block is drawn in.
    fn block_frame(&self, card: &LeagueCard, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let theme = self.theme;
        let league_id = card.league_id.clone();
        div()
            .id(SharedString::from(format!("league-{league_id}")))
            .flex()
            .flex_col()
            .gap(theme::METER_GAP)
            .px(theme::ROW_PAD_X)
            .py(theme::ROW_PAD_Y)
            .rounded(theme::ROW_RADIUS)
            .cursor_pointer()
            .hover(|style| style.bg(theme.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_league(league_id.clone(), cx)))
    }

    /// Line one: the league on the left, the two scores on the right. The
    /// higher score is ink and the lower is grey — the whole of how the
    /// popover says who is ahead. A tie gives neither of them the ink, which
    /// is the truthful drawing of a tie.
    ///
    /// With nobody on the other side the scores are replaced by the week's own
    /// word ("Bye", "No matchup"), so the block is one honest line rather than
    /// a matchup made of zeros.
    fn score_line(&self, card: &LeagueCard, ink: Ink) -> impl IntoElement {
        let name = div()
            .flex_1()
            .truncate()
            .text_color(ink.primary)
            .child(card.name.clone());
        let row = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_baseline()
            .gap(theme::METER_GAP)
            .text_size(theme::TEXT_BODY)
            .line_height(theme::LINE_TITLE)
            .child(name);

        let Some(opponent) = card.opponent.as_ref() else {
            return row.child(
                div()
                    .flex_shrink_0()
                    .text_color(ink.secondary)
                    .child(card.state.caption().unwrap_or(NO_GAME_CAPTION)),
            );
        };

        let (mine_leads, theirs_lead) = leaders(card.me.score, opponent.score);
        row.child(
            div()
                .flex_shrink_0()
                .flex()
                .flex_row()
                // Tabular digits: the scores move on every refresh and
                // proportional ones make the column jitter.
                .font_family(theme::MONO_FAMILY)
                .child(
                    div()
                        .font_weight(if mine_leads {
                            theme::WEIGHT_EMPHASIS
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(ink.score(mine_leads))
                        .child(card.me.score_text()),
                )
                .child(div().text_color(ink.secondary).child(SCORE_DASH))
                .child(
                    div()
                        .font_weight(if theirs_lead {
                            theme::WEIGHT_EMPHASIS
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(ink.score(theirs_lead))
                        .child(opponent.score_text()),
                ),
        )
    }

    /// Line two: the meter and the percentage beside it.
    ///
    /// The meter fills left to right by *my* win probability against the
    /// separator colour as its track, so a short bar reads as losing without a
    /// second colour ever being introduced. It is drawn from the same whole
    /// number that is printed next to it, so the two can never disagree.
    fn meter_line(&self, percent: u32, ink: Ink) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme::METER_GAP)
            .h(theme::LINE_TINY)
            .child(
                div()
                    .flex_1()
                    .h(theme::METER_HEIGHT)
                    .rounded(theme::METER_RADIUS)
                    .bg(self.theme.separator)
                    .overflow_hidden()
                    .child(
                        div()
                            .w(relative(meter_fraction(percent)))
                            .h_full()
                            .rounded(theme::METER_RADIUS)
                            .bg(ink.meter),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(theme::TEXT_TINY)
                    .line_height(theme::LINE_TINY)
                    .text_color(ink.secondary)
                    .child(percent_text(percent)),
            )
    }

    /// The block under the header: one entry per league, or the single line
    /// that explains why there are none.
    fn leagues_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let leagues = self.leagues();
        let frame = div()
            .flex()
            .flex_col()
            .gap(theme::BLOCK_GAP)
            .pt(px(LEAGUES_PAD_TOP))
            .pb(px(LEAGUES_PAD_BOTTOM));

        if leagues.is_empty() {
            return frame.child(
                div()
                    .px(theme::ROW_PAD_X)
                    .h(theme::LINE_TITLE)
                    .text_size(theme::TEXT_BODY)
                    .line_height(theme::LINE_TITLE)
                    .text_color(theme.secondary)
                    .truncate()
                    .child(self.empty_line()),
            );
        }
        frame.children(
            leagues
                .iter()
                .map(|card| self.league_block(card, cx))
                .collect::<Vec<_>>(),
        )
    }

    /// What stands in for the blocks when there are none. Each of these is a
    /// different thing to do next, which is why they are four lines and not
    /// one "nothing to show".
    fn empty_line(&self) -> &'static str {
        if !self.is_configured() {
            NO_USERNAME_LINE
        } else if self.provider.is_fetching() {
            LOADING_LINE
        } else if self.provider.error().is_some() {
            // The notice line above already says what failed.
            NO_SCORES_LINE
        } else {
            NO_LEAGUES_LINE
        }
    }

    // ── Menu rows ───────────────────────────────────────────────────────────

    /// A plain menu row: a label with a hover wash, like a menu item.
    fn menu_row(
        &self,
        id: SharedString,
        label: SharedString,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Div {
        self.row(id, label, Trailing::None, false, true, cx, action)
    }

    /// A settings row with a checkmark when it is the chosen one. The chosen
    /// row is drawn disabled: these are radio buttons, and clearing one would
    /// leave the setting with no value at all.
    fn choice_row(
        &self,
        id: SharedString,
        label: SharedString,
        chosen: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Div {
        self.row(
            id,
            label,
            Trailing::Check(chosen),
            true,
            !chosen,
            cx,
            action,
        )
    }

    /// Every clickable row in the popover goes through this one builder, so
    /// they all wash, indent and truncate the same way.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        id: SharedString,
        label: SharedString,
        trailing: Trailing,
        indented: bool,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Div {
        let theme = self.theme;
        let row = row_frame()
            .id(id)
            .justify_between()
            .items_center()
            .when(indented, |el| el.pl(theme::ROW_PAD_X + theme::ROW_INDENT))
            .rounded(theme::ROW_RADIUS)
            .text_size(theme::TEXT_BODY)
            .line_height(theme::LINE_TITLE)
            .text_color(if enabled { theme.text } else { theme.tertiary })
            .when(enabled, |el| {
                el.cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            })
            .child(div().flex_1().truncate().child(label))
            .children(trailing.element());
        // The rows live in a flex column, and gpui's `Stateful<Div>` is not a
        // `Div`; wrapping keeps every child of the column the same type.
        div().flex().flex_col().child(row)
    }

    /// A note under a row (no bundle, a failed toggle, or where the username
    /// is kept). It takes the indent of the row it belongs to, so it reads as
    /// that row's own small print rather than as a line of its own.
    fn note(&self, message: SharedString, indented: bool) -> impl IntoElement {
        div()
            .when(indented, |el| el.pl(theme::ROW_PAD_X + theme::ROW_INDENT))
            .when(!indented, |el| el.pl(theme::ROW_PAD_X))
            .pr(theme::ROW_PAD_X)
            .pb(px(NOTE_PAD_BOTTOM))
            .text_size(theme::TEXT_MICRO)
            .line_height(theme::LINE_MICRO)
            .text_color(self.theme.tertiary)
            .truncate()
            .child(message)
    }

    /// The rows the Settings disclosure shows in place: the username, what
    /// the menu bar item prints, and how often the scores are refetched.
    fn settings_rows(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.provider.settings();
        div()
            .flex()
            .flex_col()
            .child(section_label(self.theme, USERNAME_SECTION))
            .child(self.username_row(&settings, cx))
            .child(self.note(CONFIG_FILE_NOTE.into(), true))
            .child(section_label(self.theme, MENU_BAR_SECTION))
            .children(
                MenuBarTitle::ALL
                    .into_iter()
                    .map(|title| {
                        self.choice_row(
                            SharedString::from(format!("row-title-{}", title.as_config_str())),
                            title.label().into(),
                            settings.menu_bar_title == title,
                            cx,
                            move |this, cx| {
                                this.update_settings(cx, |settings| settings.menu_bar_title = title)
                            },
                        )
                        .into_any_element()
                    })
                    .collect::<Vec<_>>(),
            )
            .child(section_label(self.theme, REFRESH_SECTION))
            .children(
                REFRESH_CHOICES
                    .into_iter()
                    .map(|(seconds, label)| {
                        self.choice_row(
                            SharedString::from(format!("row-refresh-{seconds}")),
                            label.into(),
                            checked_refresh(settings.refresh_seconds) == seconds,
                            cx,
                            move |this, cx| {
                                this.update_settings(cx, |settings| {
                                    settings.refresh_seconds = seconds
                                })
                            },
                        )
                        .into_any_element()
                    })
                    .collect::<Vec<_>>(),
            )
    }

    /// The username row: the name Sleeper is asked about, or "Not set". It
    /// opens the config file, which is where the value actually lives.
    fn username_row(&self, settings: &Settings, cx: &mut Context<Self>) -> Div {
        let username = settings.username().unwrap_or(NO_USERNAME_VALUE).to_owned();
        self.row(
            "row-username".into(),
            username.into(),
            Trailing::None,
            true,
            true,
            cx,
            |this, cx| this.open_config_file(cx),
        )
    }

    /// The one path a settings row takes to change a setting: read what the
    /// provider holds, change the one field, hand it back. The provider
    /// persists it and re-renders both the popover and the menu bar item, so
    /// there is nothing to copy into this view.
    ///
    /// The section collapses afterwards. A menu does one thing per click, and
    /// leaving it open would hide the blocks the change was made for.
    fn update_settings(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.provider.settings();
        change(&mut settings);
        // Belt for a hand-edited file rather than a path the rows can take:
        // every interval offered is above the floor.
        settings.refresh_seconds = settings.refresh_seconds.max(MIN_REFRESH_SECONDS);
        self.provider.set_settings(settings);
        self.settings_open = false;
        cx.notify();
    }

    /// The rows under the last separator: the actions, the Settings
    /// disclosure, and Quit under a separator of its own.
    fn menu_rows(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                self.menu_row("row-refresh".into(), "Refresh".into(), cx, |this, cx| {
                    this.refresh(cx)
                }),
            )
            .child(self.menu_row(
                "row-sleeper".into(),
                "Open Sleeper".into(),
                cx,
                |this, cx| this.open_sleeper(cx),
            ))
            .child(
                self.menu_row("row-settings".into(), "Settings".into(), cx, |this, cx| {
                    this.toggle_settings(cx)
                }),
            )
            .children(self.settings_open.then(|| self.settings_rows(cx)))
            .child(separator(self.theme))
            .child(
                self.menu_row("row-quit".into(), "Quit Scorebar".into(), cx, |_, cx| {
                    cx.quit()
                }),
            )
    }
}

// ── Shared row shapes ────────────────────────────────────────────────────────

/// The frame every row in the popover shares: the menu's text inset and a
/// couple of pixels of air above and below.
pub fn row_frame() -> Div {
    div()
        .flex()
        .flex_row()
        .px(theme::ROW_PAD_X)
        .py(theme::ROW_PAD_Y)
}

/// The small tertiary label over a group of settings rows.
fn section_label(theme: Theme, label: &'static str) -> impl IntoElement {
    div()
        .px(theme::ROW_PAD_X + theme::ROW_INDENT)
        .pt(px(SECTION_LABEL_PAD_TOP))
        .pb(px(SECTION_LABEL_PAD_BOTTOM))
        .text_size(theme::TEXT_MICRO)
        .line_height(theme::LINE_MICRO)
        .text_color(theme.tertiary)
        .child(label)
}

/// A menu separator: a hairline inset from both edges with air around it.
fn separator(theme: Theme) -> Div {
    div()
        .h(theme::HAIRLINE)
        .mx(theme::SEPARATOR_INSET)
        .my(theme::SEPARATOR_MARGIN)
        .bg(theme.separator)
}

/// What sits at the right-hand end of a row.
#[derive(Debug, Clone)]
enum Trailing {
    /// Nothing — a plain menu row.
    None,
    /// A checkmark when the flag is set, and the space it would take when it
    /// is not, so a group of choices does not shuffle as the tick moves.
    Check(bool),
}

impl Trailing {
    fn element(&self) -> Option<Div> {
        match self {
            Trailing::None => None,
            Trailing::Check(on) => {
                Some(
                    div()
                        .flex_shrink_0()
                        .child(if *on { CHECKMARK } else { "" }),
                )
            }
        }
    }
}

/// The three inks a league block is drawn in, and the fill of its meter.
///
/// A stale block is every one of them moved a shade back: what was ink becomes
/// secondary, what was secondary becomes tertiary. That is the whole of
/// "dimmed" — no separate palette, no opacity, and nothing added to the theme,
/// because the popover already has three levels of grey and stale numbers are
/// exactly one level quieter than live ones.
#[derive(Debug, Clone, Copy)]
struct Ink {
    primary: Rgba,
    secondary: Rgba,
    tertiary: Rgba,
    meter: Rgba,
}

impl Ink {
    fn new(theme: Theme, stale: bool) -> Self {
        if stale {
            Self {
                primary: theme.secondary,
                secondary: theme.tertiary,
                tertiary: theme.tertiary,
                meter: theme.secondary,
            }
        } else {
            Self {
                primary: theme.text,
                secondary: theme.secondary,
                tertiary: theme.tertiary,
                meter: theme.meter_fill(),
            }
        }
    }

    /// The ink a score is set in: full strength for the side that is ahead.
    fn score(&self, leading: bool) -> Rgba {
        if leading {
            self.primary
        } else {
            self.secondary
        }
    }
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/// Which of the two scores gets the ink and the semibold, as
/// `(mine, theirs)`. A tie gives it to neither: nobody is ahead, and pointing
/// at one of them would be a lie the popover tells every Sunday morning.
fn leaders(mine: f32, theirs: f32) -> (bool, bool) {
    (mine > theirs, theirs > mine)
}

/// How far the meter is filled, from the same whole number printed beside it.
fn meter_fraction(percent: u32) -> f32 {
    (percent as f32 / 100.0).clamp(0.0, 1.0)
}

/// The win probability as the popover prints it.
fn percent_text(percent: u32) -> String {
    format!("{percent}%")
}

/// The projection caption: both projected finals, mine first, in the same
/// order as the scores above them.
fn projection_text(me: &Side, opponent: &Side) -> String {
    format!(
        "{PROJ_PREFIX}{}{SCORE_DASH}{}",
        me.projected_text(),
        opponent.projected_text()
    )
}

/// The header's left-hand side. Before the first snapshot there is no week to
/// name, so the app names itself instead of printing "Week 0".
fn week_label(week: Option<u8>) -> String {
    match week {
        Some(week) => format!("Week {week}"),
        None => APP_NAME.to_owned(),
    }
}

/// The header's right-hand side, spelled out rather than localised: the rest
/// of the popover is English, and a half-translated menu is worse than an
/// untranslated one.
fn day_label(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// The notice line: what the last fetch failed with, and — when there are
/// still numbers on screen — when those numbers are from.
///
/// Both halves matter. The failure alone leaves the user wondering whether the
/// scores are live, and the timestamp alone leaves them wondering why it has
/// stopped moving.
fn notice_text(
    error: Option<&str>,
    fetched_at: Option<i64>,
    now: DateTime<Local>,
) -> Option<String> {
    let error = error?;
    Some(match fetched_at {
        Some(at) => format!("{error}{NOTICE_JOIN}{}", moment_text(at, now)),
        None => error.to_owned(),
    })
}

/// When a snapshot was taken, as short as it can be said: a time on its own
/// today, and the weekday in front of it any other day — a popover showing
/// Sunday's scores on Tuesday has to say so.
fn moment_text(fetched_at: i64, now: DateTime<Local>) -> String {
    let Some(when) = DateTime::from_timestamp(fetched_at, 0) else {
        return UNKNOWN_MOMENT.to_owned();
    };
    let when = when.with_timezone(&Local);
    if when.date_naive() == now.date_naive() {
        format!("from {}", when.format("%-l:%M %p"))
    } else {
        format!("from {}", when.format("%a %-l:%M %p"))
    }
}

/// Which interval row carries the checkmark. The offered intervals are a short
/// list and the file can hold anything, so the largest offered interval at or
/// below what is set gets the tick — a hand-edited `refresh_seconds = 120`
/// shows as "1 minute" rather than as nothing at all.
fn checked_refresh(seconds: u64) -> u64 {
    REFRESH_CHOICES
        .iter()
        .map(|(offered, _)| *offered)
        .filter(|offered| *offered <= seconds)
        .max()
        .unwrap_or(REFRESH_CHOICES[0].0)
}

/// How tall one league's block is: three lines with a game on, one line
/// without.
fn block_height(card: &LeagueCard) -> f32 {
    if card.opponent.is_some() {
        LEAGUE_BLOCK_HEIGHT
    } else {
        ROW_HEIGHT
    }
}

/// A `file://` url for the config file, which is what the username row opens.
/// `None` on the odd machine with no home directory.
fn config_url() -> Option<String> {
    Settings::path().map(|path| format!("file://{}", path.display()))
}

// ── Layout constants ─────────────────────────────────────────────────────────

/// `4px 10px 2px` around the 13px header line.
const SECTION_HEADER_PAD_TOP: f32 = 4.0;
const SECTION_HEADER_PAD_BOTTOM: f32 = 2.0;
pub const SECTION_HEADER_HEIGHT: f32 =
    SECTION_HEADER_PAD_TOP + theme::LINE_TITLE_PX + SECTION_HEADER_PAD_BOTTOM;
/// One row: 3px of vertical padding around a 13px line.
pub const ROW_HEIGHT: f32 = theme::ROW_PAD_Y_PX * 2.0 + theme::LINE_TITLE_PX;
/// A separator and the air above and below it.
pub const SEPARATOR_HEIGHT: f32 = theme::HAIRLINE_PX + theme::SEPARATOR_MARGIN_PX * 2.0;
/// The popover's own inset, top and bottom.
const POPOVER_PAD_TOTAL: f32 = theme::POPOVER_PAD_PX * 2.0;
/// `2px … 6px` around the league blocks.
const LEAGUES_PAD_TOP: f32 = 2.0;
const LEAGUES_PAD_BOTTOM: f32 = 6.0;
/// A league with a game on: the score line, the meter line and the projection
/// caption, 4px apart, inside the padding its hover wash needs.
const LEAGUE_BLOCK_HEIGHT: f32 = theme::ROW_PAD_Y_PX * 2.0
    + theme::LINE_TITLE_PX
    + theme::METER_GAP_PX
    + theme::LINE_TINY_PX
    + theme::METER_GAP_PX
    + theme::LINE_TINY_PX;
/// The muted status line under the header.
pub const NOTICE_HEIGHT: f32 = theme::LINE_TINY_PX;
/// A note under a row.
const NOTE_PAD_BOTTOM: f32 = 4.0;
const NOTE_HEIGHT: f32 = theme::LINE_MICRO_PX + NOTE_PAD_BOTTOM;
/// A label over a group of settings rows.
const SECTION_LABEL_PAD_TOP: f32 = 6.0;
const SECTION_LABEL_PAD_BOTTOM: f32 = 2.0;
const SECTION_LABEL_HEIGHT: f32 =
    SECTION_LABEL_PAD_TOP + theme::LINE_MICRO_PX + SECTION_LABEL_PAD_BOTTOM;
/// The expanded Settings section: three labelled groups — the username row and
/// the note under it, the three menu bar choices, and the refresh intervals.
const SETTINGS_HEIGHT: f32 = 3.0 * SECTION_LABEL_HEIGHT
    + NOTE_HEIGHT
    + (1.0 + MenuBarTitle::ALL.len() as f32 + REFRESH_CHOICES.len() as f32) * ROW_HEIGHT;

// ── Strings ──────────────────────────────────────────────────────────────────

/// The name in the header before the first week lands, and in the Quit row.
const APP_NAME: &str = "Scorebar";
/// Between the two scores, and between the two projections: an en dash with a
/// space either side, which is how a scoreline is set.
const SCORE_DASH: &str = " – ";
/// The projection caption's lead-in.
const PROJ_PREFIX: &str = "proj ";
/// Between the failure and when the numbers are from.
const NOTICE_JOIN: &str = " · ";
/// What a block says when the week has no game in it and the state has no word
/// of its own.
const NO_GAME_CAPTION: &str = "No game";
/// The trailing tick on a chosen row.
const CHECKMARK: &str = "\u{2713}";
/// Section labels in the Settings disclosure.
const USERNAME_SECTION: &str = "Sleeper username";
const MENU_BAR_SECTION: &str = "Menu bar shows";
const REFRESH_SECTION: &str = "Refresh every";
/// The username row with nothing in it yet.
const NO_USERNAME_VALUE: &str = "Not set";
/// Under the username row: where the value is kept, in a path a person can
/// find. Written with a tilde rather than a home directory, which is both
/// shorter and the same on every machine.
const CONFIG_FILE_NOTE: &str = "Opens ~/.config/scorebar/config.toml";
/// What stands in for the league blocks when there are none.
const NO_USERNAME_LINE: &str = "Add your Sleeper username below";
const LOADING_LINE: &str = "Checking your leagues…";
const NO_SCORES_LINE: &str = "No scores yet";
const NO_LEAGUES_LINE: &str = "No leagues this season";
/// A timestamp that is not a moment in time — a corrupt cache, in practice.
const UNKNOWN_MOMENT: &str = "from earlier";
/// The refresh intervals the Settings section offers, in seconds, with the
/// labels it prints. One minute is the default and the cdn's own cache window;
/// the two longer ones are for a machine on a metered connection or a week
/// that has stopped moving.
const REFRESH_CHOICES: [(u64, &str); 3] =
    [(60, "1 minute"), (300, "5 minutes"), (900, "15 minutes")];

impl EventEmitter<PopoverEvent> for Popover {}

impl Focusable for Popover {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Popover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Follow the system appearance without the views having to ask: the
        // window only repaints when something notifies it, so a light/dark flip
        // while the popover is up has to wake it explicitly.
        if self.appearance.is_none() {
            let this = cx.entity();
            self.appearance = Some(window.observe_window_appearance(move |_window, cx| {
                this.update(cx, |_, cx| cx.notify());
            }));
        }
        self.theme = Theme::for_appearance(window.appearance());
        let theme = self.theme;

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(Self::on_dismiss))
            .flex()
            .flex_col()
            .w(theme::POPOVER_WIDTH)
            .h_full()
            .p(theme::POPOVER_PAD)
            .bg(theme.bg)
            .rounded(theme::POPOVER_RADIUS)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .font_family(theme::UI_FAMILY)
            .text_size(theme::TEXT_BODY)
            .text_color(theme.text)
            .child(self.header())
            .children(self.notice_line())
            .child(self.leagues_block(cx))
            .child(separator(theme))
            .child(self.menu_rows(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use scorebar_core::WeekState;
    use sleeper::RosterId;

    fn side(score: f32, projected: f32) -> Side {
        Side {
            roster_id: RosterId(1),
            team_name: "Team".to_owned(),
            record: "1-0".to_owned(),
            score,
            projected,
            yet_to_play: 2,
        }
    }

    fn card(state: WeekState, opponent: Option<Side>) -> LeagueCard {
        LeagueCard {
            league_id: LeagueId::from("league"),
            name: "League".to_owned(),
            me: side(56.92, 131.6),
            opponent,
            win_probability: 0.63,
            state,
        }
    }

    /// Noon today, so a test that subtracts hours from it stays on the same
    /// date whatever time the suite is run at.
    fn noon() -> DateTime<Local> {
        Local
            .from_local_datetime(
                &Local::now()
                    .date_naive()
                    .and_hms_opt(12, 0, 0)
                    .expect("noon exists"),
            )
            .single()
            .expect("noon is not ambiguous")
    }

    #[test]
    fn the_rows_are_the_menus_row() {
        assert_eq!(ROW_HEIGHT, 23.0);
        assert_eq!(SECTION_HEADER_HEIGHT, 23.0);
        // A 1px rule with 5px of margin either side.
        assert_eq!(SEPARATOR_HEIGHT, 11.0);
    }

    /// A block is its three lines plus the padding its hover wash needs, and a
    /// league with no game on is one plain row.
    #[test]
    fn a_league_block_is_its_score_meter_and_projection() {
        // 3 + 17 + 4 + 14 + 4 + 14 + 3.
        assert_eq!(LEAGUE_BLOCK_HEIGHT, 59.0);
        assert_eq!(
            block_height(&card(WeekState::InProgress, Some(side(2.0, 3.0)))),
            59.0
        );
        assert_eq!(block_height(&card(WeekState::Bye, None)), ROW_HEIGHT);
    }

    /// The whole popover, in the state the design was drawn for: three
    /// leagues, no notice, Settings collapsed.
    #[test]
    fn three_leagues_come_to_the_drawn_height() {
        let leagues = LEAGUES_PAD_TOP
            + 3.0 * LEAGUE_BLOCK_HEIGHT
            + 2.0 * theme::BLOCK_GAP_PX
            + LEAGUES_PAD_BOTTOM;
        assert_eq!(leagues, 203.0);
        // Refresh, Open Sleeper, Settings, Quit. There is no login item.
        let menu = 4.0 * ROW_HEIGHT + SEPARATOR_HEIGHT;
        let total = POPOVER_PAD_TOTAL + SECTION_HEADER_HEIGHT + leagues + SEPARATOR_HEIGHT + menu;
        assert_eq!(total, 350.0);
    }

    /// The notice line and the expanded Settings section each add exactly
    /// their own height, and nothing else moves.
    #[test]
    fn the_states_add_their_own_height() {
        assert_eq!(NOTICE_HEIGHT, theme::LINE_TINY_PX);
        // Three labels, the note, and the eight rows under them: the
        // username, four menu bar titles and three refresh intervals.
        assert_eq!(SETTINGS_HEIGHT, 3.0 * 21.0 + 17.0 + 8.0 * 23.0);
        const { assert!(SETTINGS_HEIGHT > 4.0 * ROW_HEIGHT) };
    }

    /// Who is ahead: ink and semibold for the leader, grey for the trailer,
    /// and neither for a tie.
    #[test]
    fn the_higher_score_leads_and_a_tie_leads_for_nobody() {
        assert_eq!(leaders(104.3, 79.0), (true, false));
        assert_eq!(leaders(79.0, 104.3), (false, true));
        assert_eq!(leaders(88.0, 88.0), (false, false));
    }

    /// The ink a block is drawn in, live and stale. Stale is one shade back
    /// everywhere, and never a colour.
    #[test]
    fn a_stale_block_is_every_ink_one_shade_back() {
        let live = Ink::new(theme::LIGHT, false);
        assert_eq!(live.score(true), theme::LIGHT.text);
        assert_eq!(live.score(false), theme::LIGHT.secondary);
        assert_eq!(live.meter, theme::LIGHT.meter_fill());

        let stale = Ink::new(theme::LIGHT, true);
        assert_eq!(stale.score(true), theme::LIGHT.secondary);
        assert_eq!(stale.score(false), theme::LIGHT.tertiary);
        assert_eq!(stale.meter, theme::LIGHT.secondary);
        assert_ne!(stale.primary, live.primary);
    }

    /// The meter is the printed percentage, so the bar and the number can
    /// never disagree, and a value out of range cannot overflow the track.
    #[test]
    fn the_meter_is_the_printed_percentage() {
        assert_eq!(meter_fraction(0), 0.0);
        assert_eq!(meter_fraction(54), 0.54);
        assert_eq!(meter_fraction(100), 1.0);
        assert_eq!(meter_fraction(140), 1.0);
        assert_eq!(percent_text(54), "54%");

        let card = card(WeekState::InProgress, Some(side(2.0, 3.0)));
        assert_eq!(card.win_percent(), 63);
        assert_eq!(meter_fraction(card.win_percent()), 0.63);
    }

    #[test]
    fn the_projection_caption_reads_as_a_scoreline() {
        let me = side(56.92, 131.64);
        let opponent = side(26.0, 79.01);
        assert_eq!(projection_text(&me, &opponent), "proj 131.6 – 79.0");
    }

    #[test]
    fn the_header_names_the_week_or_the_app() {
        assert_eq!(week_label(Some(2)), "Week 2");
        assert_eq!(week_label(Some(14)), "Week 14");
        assert_eq!(week_label(None), APP_NAME);
        assert_eq!(day_label(Weekday::Sun), "Sunday");
        assert_eq!(day_label(Weekday::Thu), "Thursday");
    }

    /// No failure, no notice: a healthy popover has nothing under its header.
    #[test]
    fn a_good_fetch_says_nothing() {
        assert_eq!(notice_text(None, Some(0), noon()), None);
        assert_eq!(notice_text(None, None, noon()), None);
    }

    /// A failure with numbers behind it says both what broke and how old they
    /// are; a failure with nothing behind it says only what broke.
    #[test]
    fn a_failed_fetch_dates_the_numbers_it_left_up() {
        let now = noon();
        let two_hours_ago = (now - chrono::Duration::hours(2)).timestamp();
        let line = notice_text(Some("Offline"), Some(two_hours_ago), now).expect("a notice");
        assert!(line.starts_with("Offline · from "), "{line}");
        assert!(line.contains("10:00"), "{line}");

        assert_eq!(
            notice_text(Some("Offline"), None, now).as_deref(),
            Some("Offline")
        );
    }

    /// Numbers from another day carry the day with them, or "1:37 PM" on a
    /// Tuesday would read as an hour ago.
    #[test]
    fn older_numbers_carry_their_weekday() {
        let now = noon();
        let today = moment_text((now - chrono::Duration::hours(1)).timestamp(), now);
        let earlier = moment_text((now - chrono::Duration::days(2)).timestamp(), now);
        assert!(!today.contains(','));
        assert_eq!(today.split_whitespace().count(), 3, "{today}");
        // "from", the weekday, the time, the meridiem.
        assert_eq!(earlier.split_whitespace().count(), 4, "{earlier}");
    }

    /// Every offered interval ticks itself, and anything else ticks the
    /// nearest one below it rather than nothing.
    #[test]
    fn the_refresh_rows_always_have_exactly_one_tick() {
        for (seconds, _) in REFRESH_CHOICES {
            assert_eq!(checked_refresh(seconds), seconds);
        }
        assert_eq!(checked_refresh(120), 60);
        assert_eq!(checked_refresh(3_600), 900);
        // Below the shortest offer — a hand-edited file — still ticks one.
        assert_eq!(checked_refresh(MIN_REFRESH_SECONDS), REFRESH_CHOICES[0].0);
        assert!(REFRESH_CHOICES
            .iter()
            .all(|(seconds, _)| *seconds >= MIN_REFRESH_SECONDS));
    }

    #[test]
    fn the_sleeper_row_opens_the_league_list() {
        assert_eq!(SLEEPER_URL, "https://sleeper.com/leagues");
    }

    /// The note has to name a path the user can find without knowing whose
    /// home directory it is.
    #[test]
    fn the_config_note_is_a_tilde_path() {
        assert!(CONFIG_FILE_NOTE.starts_with("Opens ~/"));
        assert!(CONFIG_FILE_NOTE.ends_with("config.toml"));
    }
}
