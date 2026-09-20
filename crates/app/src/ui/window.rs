//! The detail window: one league's matchup, drawn in full.
//!
//! The popover is a menu — 260px, a block per league, no lineups. This is the
//! other half of the app: an ordinary titled, resizable macOS window that the
//! popover opens when a league is worth looking at properly. It is the same
//! view system, the same theme and the same monochrome language; what changes
//! is that there is room, so the starters get a row each.
//!
//! Three things shape the layout:
//!
//! - **The tabs live in the title bar.** A manager in four leagues wants one
//!   window, not four, and macOS already gives a window a strip along the top
//!   that says which one you are looking at. Drawing plain-text tabs into a
//!   transparent title bar keeps that strip doing its job and costs no
//!   vertical space, which is why [`window_options`] hides the system title
//!   rather than adding a tab bar under it.
//! - **The header is the popover's block, widened.** Name and score, a 4px
//!   meter of win probability, the opponent under it, and an 11px line of
//!   projections. Somebody who has been reading the popover all afternoon
//!   should recognise it immediately.
//! - **A lineup row is two mirrored halves around a slot label.** The slot is
//!   what makes two players comparable, so it sits between them, and it is
//!   printed from the league's own `roster_positions` — a superflex league
//!   says `SUPERFLEX`, not `FLEX`, because that is the thing the manager
//!   picked the player for.
//!
//! Who is ahead is ink against secondary grey, per slot and in the header,
//! exactly as in the popover; see [`Theme::score_ink`]. No colour carries
//! meaning here either.
//!
//! What this view does *not* do is fetch. It renders [`LeagueDetail`] values
//! it is handed, so the whole window can be built and asserted on with no
//! network, no username and no AppKit — and so the lineup rows can carry a
//! status line the data layer has not learnt to produce yet (see
//! [`PlayerLine::status`]).

use gpui::prelude::FluentBuilder;
use gpui::{
    actions, div, px, relative, size, AnyElement, App, AppContext, Bounds, Context, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Pixels, Render, Rgba, SharedString, StatefulInteractiveElement, Styled, TitlebarOptions,
    Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use scorebar_core::{LeagueCard, Side};
use sleeper::LeagueId;

use crate::ui::theme::{self, Theme};

// ── What the window draws ────────────────────────────────────────────────────

/// One league's week, at the depth this window shows it.
///
/// [`LeagueCard`] is the popover's shape and carries everything above the
/// hairline — the two teams, the scores, the projections, the win
/// probability. It has no lineup, because the popover has no room for one, so
/// that is the only thing this type adds. Composition rather than a parallel
/// struct: a detail window that redefined "a team's score" could disagree with
/// the menu bar about the same number, and there is no version of that which
/// is not a bug.
#[derive(Debug, Clone, PartialEq)]
pub struct LeagueDetail {
    /// The header's numbers, and the league name the tab is printed from.
    pub card: LeagueCard,
    /// The starting lineup, in the league's own slot order. Bench slots are
    /// not here: the window draws what was started.
    pub slots: Vec<SlotRow>,
}

impl LeagueDetail {
    /// The league's id, which is how a tab is addressed and how a refresh
    /// finds the league the window is already showing.
    pub fn league_id(&self) -> &LeagueId {
        &self.card.league_id
    }
}

/// One slot of the lineup, with both managers' players in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotRow {
    /// The slot, spelled as the league spells it in `roster_positions`:
    /// `"QB"`, `"WR"`, `"SUPER_FLEX"`. Kept raw so the label is the league's
    /// own word for the slot rather than this view's guess at it; see
    /// [`slot_label`].
    pub position: String,
    /// The user's player in this slot. `None` for a slot left empty.
    pub mine: Option<PlayerLine>,
    /// The opponent's player in the same slot. `None` on a bye, and for a
    /// slot they left empty.
    pub theirs: Option<PlayerLine>,
}

/// One player in one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerLine {
    /// The name as it is printed, already joined: `"J. Player"`.
    pub name: String,
    /// The NFL team abbreviation, `"SF"`. Empty for a player without one.
    pub team: String,
    /// Points scored this week, or `None` for a player who has not played —
    /// which the row draws as an en dash rather than as `0.00`, because a
    /// zero that has not happened yet and a zero that has are not the same
    /// number.
    pub points: Option<f32>,
    /// The 11px line under the row: a kickoff time before the game, a live
    /// stat line during it.
    ///
    /// TODO: nothing produces this yet. The `sleeper` crate's matchups
    /// endpoint carries points and nothing else — no clock, no kickoff, no
    /// stat line — so the text has to come from a game-state source, which
    /// is Sleeper's undocumented `GET /scores/nfl/<season_type>/<season>/
    /// <week>` (what their own web client polls for game status). Until that
    /// exists the field is whatever the caller passes, empty included, and
    /// the row simply leaves the line blank.
    pub status: String,
}

// ── The pure helpers the layout is built from ────────────────────────────────

/// Which half of a slot — or of the header — is ahead.
///
/// A tie is [`Leader::Neither`] rather than a coin toss, so both sides fall
/// back to secondary grey and the row points at nobody. That is the truthful
/// drawing of a tie, and it is the same rule [`Theme::score_ink`] is built to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leader {
    /// The user's player or team.
    Mine,
    /// The opponent's.
    Theirs,
    /// Level, or nothing to compare yet.
    Neither,
}

/// Who is ahead, given each side's points.
///
/// `None` is a player who has not played. It does not compare as zero: a
/// player yet to kick off is not losing to an opponent who has also not
/// scored, and a row that inked one of them would be saying something the
/// afternoon has not decided. So a side only leads on a score it actually
/// has, and only when that score is above zero.
pub fn leader(mine: Option<f32>, theirs: Option<f32>) -> Leader {
    match (mine, theirs) {
        (Some(a), Some(b)) if a > b => Leader::Mine,
        (Some(a), Some(b)) if b > a => Leader::Theirs,
        (Some(a), None) if a > 0.0 => Leader::Mine,
        (None, Some(b)) if b > 0.0 => Leader::Theirs,
        _ => Leader::Neither,
    }
}

/// The three columns a lineup row is laid out in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Columns {
    /// Width of one player's half. Both halves get the same, so the two
    /// score columns stay symmetrical about the slot label.
    pub side: f32,
    /// Width of the slot label between them.
    pub slot: f32,
}

/// Work out the columns for a window of `width`.
///
/// Arithmetic rather than a flex rule because the name budget
/// ([`name_budget`]) has to be known before the text is laid out — the same
/// reason claudebar's popover adds its row heights up rather than measuring
/// them. The slot label is a fixed column so that the scores line up down the
/// window instead of drifting with the longest name; it is capped at a third
/// of the content so a very narrow window does not end up as a column of
/// `SUPERFLEX` with two slivers beside it.
pub fn columns(width: f32) -> Columns {
    let content = (width - WINDOW_PAD_X_PX * 2.0).max(MIN_CONTENT_PX);
    let slot = SLOT_COLUMN_PX.min(content / 3.0);
    let side = ((content - slot) / 2.0).max(MIN_SIDE_PX);
    Columns { side, slot }
}

/// How many characters of a player's name a half of `side` pixels can hold.
///
/// The score and team columns are spoken for, and what is left is divided by
/// the average advance of the 13px ui font. An approximation, deliberately:
/// it only has to be close enough that [`truncate`] cuts before the score
/// column does, and a name that has been cut a character early reads fine
/// while one that has run under the score does not.
pub fn name_budget(side: f32) -> usize {
    let room = side - SCORE_COLUMN_PX - TEAM_COLUMN_PX - COLUMN_GAP_PX * 2.0;
    let chars = (room / NAME_CHAR_PX).floor();
    if chars < MIN_NAME_CHARS as f32 {
        MIN_NAME_CHARS
    } else {
        chars as usize
    }
}

/// Cut `text` to `max` characters, ending in an ellipsis when it had to cut.
///
/// Counted in characters, not bytes, so a name with an accent in it is cut in
/// the right place rather than panicking. The ellipsis is one character and
/// replaces the last one kept, so the result is never wider than the budget.
pub fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", kept.trim_end())
}

/// The label printed between two players, from the league's own slot name.
///
/// Sleeper spells its slots in screaming snake case — `SUPER_FLEX`,
/// `REC_FLEX`, `IDP_FLEX` — which is a wire format, not a label. The
/// underscore goes, and the handful of slots whose printed name is a single
/// word get it. Anything unrecognised is passed through with its underscores
/// turned into spaces, so a league with a slot this list has never heard of
/// still gets its own word rather than a blank column.
pub fn slot_label(position: &str) -> String {
    match position {
        "SUPER_FLEX" => "SUPERFLEX".to_owned(),
        "WRRB_FLEX" => "FLEX".to_owned(),
        "REC_FLEX" => "REC FLEX".to_owned(),
        "IDP_FLEX" => "IDP".to_owned(),
        "DEF" | "DST" => "DST".to_owned(),
        other => other.replace('_', " "),
    }
}

/// A score as the column prints it, or an en dash for a player who has not
/// played. Two decimals, the same as everywhere else in the app: fantasy
/// games are decided in hundredths.
pub fn points_text(points: Option<f32>) -> String {
    match points {
        Some(points) => format!("{points:.2}"),
        None => NOT_PLAYED.to_owned(),
    }
}

/// The 11px line under one side of the header: `"proj 131.6 · 5 to play"`.
///
/// The count is dropped once everybody has played, because "0 to play" is a
/// sentence about nothing; what is left is the projection, which by then has
/// stopped being a projection and is worth seeing next to the final.
pub fn projection_line(side: &Side) -> String {
    if side.yet_to_play == 0 {
        format!("proj {}", side.projected_text())
    } else {
        format!(
            "proj {} · {} to play",
            side.projected_text(),
            side.yet_to_play
        )
    }
}

/// The centre column of the header caption: `"96% win"`.
pub fn win_line(card: &LeagueCard) -> String {
    format!("{}% win", card.win_percent())
}

// ── The view ─────────────────────────────────────────────────────────────────

/// What the detail window asks its owner to do.
pub enum DetailEvent {
    /// Esc, or the window closing itself — the owner should drop its handle.
    Close,
}

actions!(scorebar, [CloseDetail, NextLeague, PreviousLeague]);

/// Key context for the detail window. Distinct from the popover's, so a
/// binding meant for one never fires in the other.
pub const KEY_CONTEXT: &str = "ScorebarDetail";

/// Install the detail window's key bindings. Call once at app start.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseDetail, Some(KEY_CONTEXT)),
        // The tabs are a row, so the arrows walk them; cmd-shift-bracket is
        // what a Mac user's hands reach for in a tabbed window.
        KeyBinding::new("right", NextLeague, Some(KEY_CONTEXT)),
        KeyBinding::new("left", PreviousLeague, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-]", NextLeague, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-[", PreviousLeague, Some(KEY_CONTEXT)),
    ]);
}

/// The root view of the detail window.
pub struct DetailWindow {
    focus: FocusHandle,
    leagues: Vec<LeagueDetail>,
    /// Which tab is showing. Always a valid index while `leagues` is not
    /// empty; [`DetailWindow::set_leagues`] is what keeps that true.
    active: usize,
    theme: Theme,
    appearance: Option<gpui::Subscription>,
}

impl DetailWindow {
    /// Build the window over the leagues it should show, opened on `active`.
    ///
    /// An out-of-range index lands on the first league rather than on
    /// nothing: the caller is usually passing the league the popover row
    /// belonged to, and a refresh that dropped that league is not a reason to
    /// show an empty window.
    pub fn new(leagues: Vec<LeagueDetail>, active: usize, cx: &mut Context<Self>) -> Self {
        let active = if active < leagues.len() { active } else { 0 };
        Self {
            focus: cx.focus_handle(),
            leagues,
            active,
            theme: Theme::default(),
            appearance: None,
        }
    }

    /// The same, opened on a league by id — what the popover has in hand.
    pub fn for_league(
        leagues: Vec<LeagueDetail>,
        league_id: &LeagueId,
        cx: &mut Context<Self>,
    ) -> Self {
        let active = index_of(&leagues, league_id).unwrap_or(0);
        Self::new(leagues, active, cx)
    }

    /// Replace the leagues after a refresh, keeping the tab the user is
    /// looking at.
    ///
    /// By id, not by index: leagues arrive in the user's own Sleeper order and
    /// that order can change between fetches, and a window that silently swaps
    /// to another league while somebody is reading it is worse than one that
    /// falls back to the first tab.
    pub fn set_leagues(&mut self, leagues: Vec<LeagueDetail>, cx: &mut Context<Self>) {
        let showing = self.active().map(|league| league.league_id().clone());
        self.active = showing
            .and_then(|id| index_of(&leagues, &id))
            .unwrap_or(0)
            .min(leagues.len().saturating_sub(1));
        self.leagues = leagues;
        cx.notify();
    }

    /// The league on screen, or `None` when there are no leagues at all.
    pub fn active(&self) -> Option<&LeagueDetail> {
        self.leagues.get(self.active)
    }

    /// Switch to a league by id. Does nothing if this window is not showing
    /// that league.
    pub fn show_league(&mut self, league_id: &LeagueId, cx: &mut Context<Self>) {
        if let Some(index) = index_of(&self.leagues, league_id) {
            self.select(index, cx);
        }
    }

    /// The theme this window is currently drawn in.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.leagues.len() && index != self.active {
            self.active = index;
            cx.notify();
        }
    }

    /// Walk the tabs. Wraps, because four tabs in a row with no scroll is a
    /// ring, and a right arrow that stops dead on the last one just feels
    /// broken.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.leagues.len();
        if count < 2 {
            return;
        }
        let next = if forward {
            (self.active + 1) % count
        } else {
            (self.active + count - 1) % count
        };
        self.select(next, cx);
    }

    fn on_close(&mut self, _: &CloseDetail, window: &mut Window, cx: &mut Context<Self>) {
        // Tell the owner first: it holds the handle, and a window that has
        // already gone cannot be updated to say so.
        cx.emit(DetailEvent::Close);
        window.remove_window();
    }

    // ── Layout ──────────────────────────────────────────────────────────────

    /// The title bar: room for the traffic lights, then one plain-text tab per
    /// league.
    ///
    /// The active tab is semibold ink and the rest are secondary — the same
    /// two weights and the same two greys the rest of the app uses, with no
    /// underline, pill or box. A tab is a word you can click; the window's own
    /// border is already saying where the content starts.
    fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(TITLEBAR_HEIGHT_PX))
            .pl(px(TITLEBAR_LEFT_INSET_PX))
            .pr(px(WINDOW_PAD_X_PX))
            .gap(px(TAB_GAP_PX))
            .children(
                self.leagues
                    .iter()
                    .enumerate()
                    .map(|(index, league)| {
                        let active = index == self.active;
                        div()
                            .id(SharedString::from(format!("tab-{index}")))
                            .cursor_pointer()
                            .text_size(theme::TEXT_TITLE)
                            .line_height(theme::LINE_TITLE)
                            .text_color(if active { theme.text } else { theme.secondary })
                            .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(SharedString::from(truncate(&league.card.name, TAB_CHARS)))
                            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>(),
            )
    }

    /// The header block: the two teams, the meter between them, and the
    /// caption row under it.
    ///
    /// The names are fixed — yours in ink because it is yours, theirs in
    /// secondary — and the scores are the part that moves: each one takes
    /// [`Theme::score_ink`], and the leader's is semibold. So in the ordinary
    /// case, where you are ahead, the block reads exactly as the popover's
    /// does, and when you fall behind the ink crosses over to their score
    /// without anything else in the block shifting.
    fn header(&self, league: &LeagueDetail) -> impl IntoElement {
        let theme = self.theme;
        let card = &league.card;
        let me = &card.me;
        let opponent = card.opponent.as_ref();
        let ahead = leader(Some(me.score), opponent.map(|side| side.score));

        let their_name: SharedString = match opponent {
            Some(side) => side.team_name.clone().into(),
            // A bye or an unpaired week has no team to name, so the state
            // says what is happening instead of a blank line pretending
            // somebody is there.
            None => card.state.caption().unwrap_or(NO_OPPONENT).into(),
        };
        let their_score: SharedString = match opponent {
            Some(side) => side.score_text().into(),
            None => NOT_PLAYED.into(),
        };
        let their_projection: SharedString = match opponent {
            Some(side) => projection_line(side).into(),
            None => SharedString::default(),
        };

        div()
            .flex()
            .flex_col()
            .px(px(WINDOW_PAD_X_PX))
            .pt(px(HEADER_PAD_TOP_PX))
            .pb(px(HEADER_PAD_BOTTOM_PX))
            .gap(theme::METER_GAP)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .justify_between()
                    .gap(px(COLUMN_GAP_PX))
                    .text_size(theme::TEXT_TITLE)
                    .line_height(theme::LINE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .truncate()
                            .text_color(theme.text)
                            .child(SharedString::from(me.team_name.clone())),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::MONO_FAMILY)
                            .text_color(theme.score_ink(ahead == Leader::Mine))
                            .child(SharedString::from(me.score_text())),
                    ),
            )
            .child(self.meter(card.win_probability))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .justify_between()
                    .gap(px(COLUMN_GAP_PX))
                    .text_size(theme::TEXT_TITLE)
                    .line_height(theme::LINE_TITLE)
                    .text_color(theme.secondary)
                    .child(div().truncate().child(their_name))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::MONO_FAMILY)
                            .when(ahead == Leader::Theirs, |el| {
                                el.font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.score_ink(true))
                            })
                            .child(their_score),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .pt(px(CAPTION_PAD_TOP_PX))
                    .text_size(theme::TEXT_TINY)
                    .line_height(theme::LINE_TINY)
                    .text_color(theme.tertiary)
                    .child(
                        div()
                            .flex_1()
                            .child(SharedString::from(projection_line(me))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_center()
                            .child(SharedString::from(win_line(card))),
                    )
                    .child(div().flex_1().text_right().child(their_projection)),
            )
    }

    /// The win probability meter: a 4px track with an ink fill.
    ///
    /// A proportion, not a verdict, so it is ink whichever way it is leaning —
    /// see [`Theme::meter_fill`]. It fills from the left because the left of
    /// this window is your side of the game.
    fn meter(&self, probability: f32) -> impl IntoElement {
        let theme = self.theme;
        div()
            .w_full()
            .h(theme::METER_HEIGHT)
            .rounded(theme::METER_RADIUS)
            .bg(theme.separator)
            .overflow_hidden()
            .child(
                div()
                    .w(relative(probability.clamp(0.0, 1.0)))
                    .h_full()
                    .rounded(theme::METER_RADIUS)
                    .bg(theme.meter_fill()),
            )
    }

    /// One lineup slot: two mirrored halves with the slot label between them.
    fn slot_row(&self, row: &SlotRow, columns: Columns) -> impl IntoElement {
        let theme = self.theme;
        let ahead = leader(
            row.mine.as_ref().and_then(|player| player.points),
            row.theirs.as_ref().and_then(|player| player.points),
        );
        div()
            .flex()
            .flex_row()
            .items_start()
            .px(px(WINDOW_PAD_X_PX))
            .py(px(SLOT_ROW_PAD_Y_PX))
            .child(self.player_half(row.mine.as_ref(), ahead == Leader::Mine, false, columns))
            .child(
                div()
                    .w(px(columns.slot))
                    .flex_shrink_0()
                    .text_center()
                    .text_size(theme::TEXT_TINY)
                    .line_height(theme::LINE_TITLE)
                    .text_color(theme.tertiary)
                    .child(SharedString::from(slot_label(&row.position))),
            )
            .child(self.player_half(row.theirs.as_ref(), ahead == Leader::Theirs, true, columns))
    }

    /// One player: the name, the team, the score, and the status line under
    /// them.
    ///
    /// `mirrored` is the opponent's half, which reads inwards — score, team,
    /// name — so that the two score columns meet in the middle of the window
    /// rather than both hugging the left. Everything else about the two halves
    /// is identical, which is the point: the row is a comparison, and a
    /// comparison drawn two different ways is not one.
    fn player_half(
        &self,
        player: Option<&PlayerLine>,
        leading: bool,
        mirrored: bool,
        columns: Columns,
    ) -> impl IntoElement {
        let theme = self.theme;
        let ink = theme.score_ink(leading);
        let (name, team, points, status) = match player {
            Some(player) => (
                truncate(&player.name, name_budget(columns.side)),
                player.team.clone(),
                points_text(player.points),
                player.status.clone(),
            ),
            // An empty slot still holds its row open, so the lineup below it
            // does not slide up past its opposite number.
            None => (
                EMPTY_SLOT.to_owned(),
                String::new(),
                NOT_PLAYED.to_owned(),
                String::new(),
            ),
        };

        let name = div()
            .text_color(ink)
            .child(SharedString::from(name))
            .into_any_element();
        let team = div()
            .flex_shrink_0()
            .text_size(theme::TEXT_TINY)
            .text_color(theme.tertiary)
            .child(SharedString::from(team))
            .into_any_element();
        let score = div()
            .flex_shrink_0()
            .font_family(theme::MONO_FAMILY)
            .text_color(ink)
            .when(leading, |el| el.font_weight(FontWeight::SEMIBOLD))
            .child(SharedString::from(points))
            .into_any_element();

        let mut named: Vec<AnyElement> = vec![name, team];
        if mirrored {
            named.reverse();
        }
        let named = div()
            .flex()
            .flex_row()
            .items_baseline()
            .gap(px(COLUMN_GAP_PX))
            .children(named)
            .into_any_element();
        // The score is the column that has to meet its opposite number in the
        // middle of the window, so it leads the mirrored half and trails the
        // other one.
        let line: Vec<AnyElement> = if mirrored {
            vec![score, named]
        } else {
            vec![named, score]
        };

        div()
            .flex()
            .flex_col()
            .w(px(columns.side))
            .flex_shrink_0()
            .gap(px(STATUS_GAP_PX))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .justify_between()
                    .gap(px(COLUMN_GAP_PX))
                    .text_size(theme::TEXT_BODY)
                    .line_height(theme::LINE_TITLE)
                    .children(line),
            )
            .child(
                div()
                    .text_size(theme::TEXT_TINY)
                    .line_height(theme::LINE_TINY)
                    .text_color(theme.tertiary)
                    .when(mirrored, |el| el.text_right())
                    .child(SharedString::from(status)),
            )
    }

    /// The line that stands in for the lineup when there is nothing to draw:
    /// no leagues at all, or a league whose starters have not been fetched.
    fn empty_line(&self, message: &'static str) -> impl IntoElement {
        div()
            .px(px(WINDOW_PAD_X_PX))
            .py(px(SLOT_ROW_PAD_Y_PX))
            .text_size(theme::TEXT_BODY)
            .line_height(theme::LINE_TITLE)
            .text_color(self.theme.secondary)
            .child(message)
    }
}

/// Where a league sits in the list, by id.
fn index_of(leagues: &[LeagueDetail], league_id: &LeagueId) -> Option<usize> {
    leagues
        .iter()
        .position(|league| league.league_id() == league_id)
}

/// A hairline across the content, inset to the window's own gutter.
fn hairline(theme: Theme) -> impl IntoElement {
    div()
        .h(theme::HAIRLINE)
        .mx(px(WINDOW_PAD_X_PX))
        .bg(theme.separator)
}

/// The window's material.
///
/// The popover's background is translucent by design — it is a menu hanging
/// off the menu bar, over a blurred panel. A titled window is not a menu and
/// does not get the blur, so the same colour is taken at full strength; see
/// [`theme::BG_ALPHA`] for why the popover's is not.
fn window_bg(theme: Theme) -> Rgba {
    Rgba { a: 1.0, ..theme.bg }
}

impl EventEmitter<DetailEvent> for DetailWindow {}

impl Focusable for DetailWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DetailWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Same as the popover: the window only repaints when something
        // notifies it, so a light/dark flip while it is open has to wake it.
        if self.appearance.is_none() {
            let this = cx.entity();
            self.appearance = Some(window.observe_window_appearance(move |_window, cx| {
                this.update(cx, |_, cx| cx.notify());
            }));
        }
        self.theme = Theme::for_appearance(window.appearance());
        let theme = self.theme;
        let columns = columns(f32::from(window.viewport_size().width));

        let titlebar = self.titlebar(cx);
        let body = match self.active() {
            Some(league) => {
                let rows: Vec<AnyElement> = league
                    .slots
                    .iter()
                    .enumerate()
                    .flat_map(|(index, row)| {
                        let mut parts: Vec<AnyElement> = Vec::with_capacity(2);
                        if index > 0 {
                            parts.push(hairline(theme).into_any_element());
                        }
                        parts.push(self.slot_row(row, columns).into_any_element());
                        parts
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .child(self.header(league))
                    .child(hairline(theme))
                    .child(
                        div()
                            .id("slots")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_y_scroll()
                            .children(if rows.is_empty() {
                                vec![self.empty_line(NO_LINEUP).into_any_element()]
                            } else {
                                rows
                            }),
                    )
                    .into_any_element()
            }
            None => self.empty_line(NO_LEAGUES).into_any_element(),
        };

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(|this, _: &NextLeague, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousLeague, _, cx| this.step(false, cx)))
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .bg(window_bg(theme))
            .overflow_hidden()
            .font_family(theme::UI_FAMILY)
            .text_size(theme::TEXT_BODY)
            .text_color(theme.text)
            .child(titlebar)
            .child(body)
    }
}

// ── Opening the window ───────────────────────────────────────────────────────

/// The options the detail window is opened with.
///
/// A normal, movable, resizable, minimizable window — everything the popover's
/// panel is not. The title bar is transparent and its title hidden so the view
/// can draw the league tabs into that strip itself; the title is still set,
/// because it is what Mission Control and the Window menu print. The traffic
/// lights are nudged down to sit on the tabs' baseline in a title bar this
/// tall.
pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(
                px(TRAFFIC_LIGHT_X_PX),
                px(TRAFFIC_LIGHT_Y_PX),
            )),
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        // Opaque, not blurred: the popover is a menu and reads as one because
        // the desktop shows through it; a document window that did the same
        // would just look unfinished.
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(MIN_WIDTH_PX), px(MIN_HEIGHT_PX))),
        display_id: None,
        app_id: None,
        window_decorations: None,
        tabbing_identifier: None,
    }
}

/// Open the detail window, centred on the main display, showing `league_id`.
///
/// The caller keeps the handle: it is how the window is closed, how a refresh
/// pushes new leagues in ([`DetailWindow::set_leagues`]), and how a second
/// click on the same league raises the window that is already open instead of
/// opening another.
///
/// Note for the owner: scorebar runs as an accessory app with no Dock icon, so
/// it is never the active application on its own. The window asks to be made
/// key here, but the app also has to be activated — otherwise this opens
/// behind whatever the user was doing.
pub fn open(
    cx: &mut App,
    leagues: Vec<LeagueDetail>,
    league_id: Option<&LeagueId>,
) -> anyhow::Result<WindowHandle<DetailWindow>> {
    let bounds = Bounds::centered(None, size(px(DEFAULT_WIDTH_PX), px(DEFAULT_HEIGHT_PX)), cx);
    let active = league_id.and_then(|id| index_of(&leagues, id)).unwrap_or(0);
    let handle = cx.open_window(window_options(bounds), |window, cx| {
        let view = cx.new(|cx| DetailWindow::new(leagues, active, cx));
        window.focus(&view.focus_handle(cx));
        view
    })?;
    handle.update(cx, |_, window, _| window.activate_window())?;
    Ok(handle)
}

// ── Layout constants ─────────────────────────────────────────────────────────
//
// The tokens — colours, the type scale, the meter, the hairline — all come
// from `theme`. What is here is this one window's arithmetic: the widths and
// paddings that only mean anything inside it, kept beside the code that adds
// them up, the way `popover.rs` keeps its section heights.

/// The gutter down both sides of the window: twice the popover's 10px row
/// inset. The popover is 260px wide and cannot spare the air; this window can,
/// and at 560px the menu's own inset would read as no margin at all.
const WINDOW_PAD_X_PX: f32 = 20.0;
/// Height of the title bar the tabs are drawn into.
const TITLEBAR_HEIGHT_PX: f32 = 38.0;
/// Where the tabs start: clear of the three traffic lights.
const TITLEBAR_LEFT_INSET_PX: f32 = 78.0;
/// The gap between two tabs. Wide enough that two plain words read as two
/// tabs without a divider between them.
const TAB_GAP_PX: f32 = 16.0;
/// A league name longer than this is cut in the tab. Long names are common —
/// leagues are named by committee — and a tab row that wrapped would push the
/// traffic lights off their own line.
const TAB_CHARS: usize = 22;
/// Traffic lights, centred in a title bar this tall.
const TRAFFIC_LIGHT_X_PX: f32 = 13.0;
const TRAFFIC_LIGHT_Y_PX: f32 = (TITLEBAR_HEIGHT_PX - 12.0) / 2.0;

/// The air around the header block.
const HEADER_PAD_TOP_PX: f32 = 6.0;
const HEADER_PAD_BOTTOM_PX: f32 = 10.0;
/// The caption row sits a little further from the opponent's line than the
/// meter does from the scores, so the block reads as a matchup with a note
/// under it rather than as four equal lines.
const CAPTION_PAD_TOP_PX: f32 = 2.0;

/// The gap between the two lines of a player's half.
const STATUS_GAP_PX: f32 = 1.0;
/// The air above and below a lineup row. More than a menu row's: these rows
/// are two lines tall and separated by hairlines, so they need the room.
const SLOT_ROW_PAD_Y_PX: f32 = 7.0;
/// The gap between the pieces of a row — name, team, score.
const COLUMN_GAP_PX: f32 = 6.0;
/// The slot label's column, wide enough for `SUPERFLEX` at 11px.
const SLOT_COLUMN_PX: f32 = 72.0;
/// The score column, wide enough for a three-figure score in 13px mono.
const SCORE_COLUMN_PX: f32 = 52.0;
/// The team abbreviation's column at 11px.
const TEAM_COLUMN_PX: f32 = 28.0;
/// Average advance of the 13px ui font, used to budget a name into what is
/// left of its half. See [`name_budget`].
const NAME_CHAR_PX: f32 = 7.0;
/// However narrow the window gets, a name keeps this many characters — below
/// it the column stops being a name and becomes an initial.
const MIN_NAME_CHARS: usize = 6;
/// A player's half never goes below this, and the window's minimum width is
/// set from it.
const MIN_SIDE_PX: f32 = 140.0;
/// The narrowest content the columns are ever divided from.
const MIN_CONTENT_PX: f32 = MIN_SIDE_PX * 2.0 + SLOT_COLUMN_PX;

/// The window's opening size, and the smallest the user can drag it to.
const DEFAULT_WIDTH_PX: f32 = 560.0;
const DEFAULT_HEIGHT_PX: f32 = 620.0;
const MIN_WIDTH_PX: f32 = MIN_CONTENT_PX + WINDOW_PAD_X_PX * 2.0;
const MIN_HEIGHT_PX: f32 = 360.0;

/// What the Window menu and Mission Control call this window.
const WINDOW_TITLE: &str = "Scorebar";
/// An en dash: a score that has not happened yet.
const NOT_PLAYED: &str = "\u{2013}";
/// A slot nobody started anyone in.
const EMPTY_SLOT: &str = "Empty";
/// Stand-ins for a window with nothing to draw.
const NO_LEAGUES: &str = "No leagues yet";
const NO_LINEUP: &str = "No lineup yet";
/// The other side of a bye.
const NO_OPPONENT: &str = "No opponent";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_halves_are_equal_and_the_slot_sits_between_them() {
        let columns = columns(DEFAULT_WIDTH_PX);
        assert_eq!(columns.slot, SLOT_COLUMN_PX);
        // Both halves plus the slot are exactly the content width.
        let content = DEFAULT_WIDTH_PX - WINDOW_PAD_X_PX * 2.0;
        assert!((columns.side * 2.0 + columns.slot - content).abs() < 1e-3);
    }

    /// Dragged narrow, the label column gives way before the players do, and
    /// neither half falls below the minimum the window is sized to keep.
    #[test]
    fn a_narrow_window_keeps_the_players_and_shrinks_the_label() {
        let narrow = columns(MIN_WIDTH_PX);
        assert!(narrow.side >= MIN_SIDE_PX);
        assert!(narrow.slot <= SLOT_COLUMN_PX);

        // Below the minimum width the columns stop shrinking rather than
        // going negative: the window cannot be dragged there, but a display
        // change could still ask.
        let tiny = columns(120.0);
        assert!(tiny.side >= MIN_SIDE_PX);
        assert!(tiny.slot > 0.0);
    }

    /// The window's minimum width is the one the columns are happy at, so the
    /// two can never disagree.
    #[test]
    fn the_minimum_width_is_the_width_the_columns_need() {
        let at_minimum = columns(MIN_WIDTH_PX);
        assert_eq!(at_minimum.side, MIN_SIDE_PX);
        assert_eq!(at_minimum.slot, SLOT_COLUMN_PX);
    }

    #[test]
    fn a_name_is_budgeted_from_what_the_score_columns_leave() {
        let wide = name_budget(400.0);
        let narrow = name_budget(MIN_SIDE_PX);
        assert!(wide > narrow);
        assert!(narrow >= MIN_NAME_CHARS);
        // A name is never budgeted into the score column.
        assert!(wide as f32 * NAME_CHAR_PX <= 400.0 - SCORE_COLUMN_PX);
    }

    #[test]
    fn the_higher_score_in_a_slot_leads() {
        assert_eq!(leader(Some(18.4), Some(6.2)), Leader::Mine);
        assert_eq!(leader(Some(6.2), Some(18.4)), Leader::Theirs);
    }

    /// A tie points at nobody, the same way the theme draws one.
    #[test]
    fn a_tie_leads_for_neither_side() {
        assert_eq!(leader(Some(9.0), Some(9.0)), Leader::Neither);
        assert_eq!(leader(Some(0.0), Some(0.0)), Leader::Neither);
        assert_eq!(leader(None, None), Leader::Neither);
    }

    /// A player yet to kick off is not behind one who has also not scored —
    /// only a score above zero puts a side ahead of an empty column.
    #[test]
    fn a_player_who_has_not_played_does_not_lose_to_a_scoreless_one() {
        assert_eq!(leader(Some(0.0), None), Leader::Neither);
        assert_eq!(leader(None, Some(0.0)), Leader::Neither);
        assert_eq!(leader(Some(4.0), None), Leader::Mine);
        assert_eq!(leader(None, Some(4.0)), Leader::Theirs);
    }

    #[test]
    fn a_name_that_fits_is_left_alone() {
        assert_eq!(truncate("J. Player", 20), "J. Player");
        assert_eq!(truncate("J. Player", 9), "J. Player");
        assert_eq!(truncate("", 4), "");
    }

    #[test]
    fn a_name_that_does_not_fit_is_cut_to_the_budget() {
        // Nine characters of budget: eight kept plus the ellipsis.
        assert_eq!(truncate("A Very Long Name", 9), "A Very L\u{2026}");
        assert_eq!(truncate("A Very Long Name", 9).chars().count(), 9);
        // A cut that lands on a space does not leave it dangling before the
        // ellipsis.
        assert_eq!(truncate("A Very Long Name", 8), "A Very\u{2026}");
        assert_eq!(truncate("anything", 0), "");
    }

    /// Counted in characters, so a name with an accent in it is cut where it
    /// looks like it should be and never panics on a byte boundary.
    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        assert_eq!(truncate("Ramírez", 7), "Ramírez");
        assert_eq!(truncate("Ramírez", 5), "Ramí\u{2026}");
    }

    /// A tab is cut the same way, so a committee-named league cannot push the
    /// row onto a second line.
    #[test]
    fn a_long_league_name_is_cut_in_its_tab() {
        let long = "A".repeat(40);
        assert_eq!(truncate(&long, TAB_CHARS).chars().count(), TAB_CHARS);
    }

    /// The label is the league's own word for the slot, so a superflex league
    /// says superflex.
    #[test]
    fn a_slot_is_labelled_the_way_its_league_spells_it() {
        assert_eq!(slot_label("QB"), "QB");
        assert_eq!(slot_label("SUPER_FLEX"), "SUPERFLEX");
        assert_eq!(slot_label("FLEX"), "FLEX");
        assert_eq!(slot_label("WRRB_FLEX"), "FLEX");
        assert_eq!(slot_label("REC_FLEX"), "REC FLEX");
        assert_eq!(slot_label("DEF"), "DST");
        // Unknown slots keep their own name rather than vanishing.
        assert_eq!(slot_label("TAXI_SPOT"), "TAXI SPOT");
    }

    #[test]
    fn a_score_that_has_not_happened_is_an_en_dash() {
        assert_eq!(points_text(Some(18.4)), "18.40");
        assert_eq!(points_text(Some(0.0)), "0.00");
        assert_eq!(points_text(None), "\u{2013}");
    }

    #[test]
    fn the_caption_drops_the_count_once_everyone_has_played() {
        let mut side = Side {
            roster_id: sleeper::RosterId(1),
            team_name: "Team One".to_owned(),
            record: "1-0".to_owned(),
            score: 96.5,
            projected: 131.62,
            yet_to_play: 5,
        };
        assert_eq!(projection_line(&side), "proj 131.6 · 5 to play");
        side.yet_to_play = 0;
        assert_eq!(projection_line(&side), "proj 131.6");
    }
}
