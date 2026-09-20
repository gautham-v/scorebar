//! The handful of user settings, and the TOML file they live in.
//!
//! scorebar works with no config at all: everything here has a default, and a
//! missing file is the normal case rather than an error. The one thing it
//! cannot guess is the Sleeper username, which is why that field is an
//! `Option` and why the popover asks for it when it is `None`.
//!
//! The file is `~/.config/scorebar/config.toml`. It is read once at startup
//! and rewritten whenever the popover's Settings section changes something; a
//! partial file is fine (serde fills the rest in from the defaults) and
//! unknown keys are ignored, so a file written by a newer build never stops an
//! older one from starting. A file that does not parse at all is not fatal
//! either: [`Settings::load`] hands back the defaults plus one human-readable
//! note, which the popover shows as its muted notice line.

use std::path::PathBuf;
use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// The directory name scorebar uses under `~/.config` and under
/// `~/Library/Caches`. One constant so the two cannot drift apart.
pub const APP_DIR_NAME: &str = "scorebar";

/// What the menu bar item prints beside the scoreboard glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuBarTitle {
    /// The closest game's score line — `"65.44 – 104.32"`. The default,
    /// because the whole reason to put a fantasy score in the menu bar is the
    /// matchup that is still in doubt.
    #[default]
    ClosestGame,
    /// A record across every league, like `"2–1"`. What to show once the week
    /// is decided and the scores have stopped moving.
    Record,
    /// The glyph on its own. The narrowest the item gets, for a menu bar with
    /// no room left in it; the popover still carries everything.
    GlyphOnly,
}

/// How each choice is spelled in the config file — plain lowercase words, so
/// the file stays something a person can edit.
const CLOSEST_GAME_KEY: &str = "closest_game";
const RECORD_KEY: &str = "record";
const GLYPH_ONLY_KEY: &str = "glyph_only";

impl MenuBarTitle {
    /// How this choice is spelled in the config file.
    pub fn as_config_str(&self) -> &'static str {
        match self {
            MenuBarTitle::ClosestGame => CLOSEST_GAME_KEY,
            MenuBarTitle::Record => RECORD_KEY,
            MenuBarTitle::GlyphOnly => GLYPH_ONLY_KEY,
        }
    }

    /// The other direction. Matched case-insensitively so a hand-edited
    /// `"Record"` still works, and anything unrecognised falls back to the
    /// default rather than refusing the file — a value this build has never
    /// heard of is most likely one a newer build wrote.
    pub fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        if trimmed.eq_ignore_ascii_case(RECORD_KEY) {
            MenuBarTitle::Record
        } else if trimmed.eq_ignore_ascii_case(GLYPH_ONLY_KEY) {
            MenuBarTitle::GlyphOnly
        } else {
            MenuBarTitle::ClosestGame
        }
    }

    /// Every choice, in the order the Settings section lists them.
    pub const ALL: [MenuBarTitle; 3] = [
        MenuBarTitle::ClosestGame,
        MenuBarTitle::Record,
        MenuBarTitle::GlyphOnly,
    ];

    /// The label the Settings row shows for this choice.
    pub fn label(&self) -> &'static str {
        match self {
            MenuBarTitle::ClosestGame => "Closest game",
            MenuBarTitle::Record => "Record",
            MenuBarTitle::GlyphOnly => "Icon only",
        }
    }
}

impl Serialize for MenuBarTitle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_config_str())
    }
}

impl<'de> Deserialize<'de> for MenuBarTitle {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.trim().is_empty() {
            return Err(de::Error::custom("menu_bar_title must not be empty"));
        }
        Ok(MenuBarTitle::parse(&value))
    }
}

/// Everything the user can choose. Every field carries a serde default, so a
/// file with one key in it is a valid file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The Sleeper username, which is the only thing scorebar needs from the
    /// user and the only thing it cannot default. Sleeper's read api is
    /// public — no key, no oauth — so a username is enough to find the
    /// leagues and everything after that is public league data.
    ///
    /// `None` until it is set, which is what the popover's "add your username"
    /// state is keyed off.
    pub sleeper_username: Option<String>,
    /// What the menu bar item prints beside the glyph.
    pub menu_bar_title: MenuBarTitle,
    /// How often the scores are refetched, in seconds.
    pub refresh_seconds: u64,
}

/// The default poll interval, in seconds.
///
/// Sleeper's cdn serves the live endpoints with `s-maxage=60`, so a refresh
/// faster than this re-reads Cloudflare's copy of the same numbers: it costs
/// requests and returns nothing new. Sixty seconds is the fastest cadence that
/// can actually see a change.
pub const DEFAULT_REFRESH_SECONDS: u64 = 60;

/// The floor a hand-edited file is clamped to. Below this the app is spending
/// requests on numbers the cdn has not refreshed yet; a `0` would be a refresh
/// loop with no delay in it at all.
pub const MIN_REFRESH_SECONDS: u64 = 15;

impl Default for Settings {
    fn default() -> Self {
        Self {
            sleeper_username: None,
            menu_bar_title: MenuBarTitle::ClosestGame,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
        }
    }
}

/// What goes at the top of a file we write, so whoever opens it knows what it
/// is and that the popover owns it.
const FILE_HEADER: &str = "\
# scorebar settings. Written by the Settings section; safe to edit by hand.
# menu_bar_title: \"closest_game\", \"record\", or \"glyph_only\".
# refresh_seconds: Sleeper's cdn caches these endpoints for 60 seconds, so
# anything faster refetches the same numbers.
";

impl Settings {
    /// `~/.config/scorebar/config.toml`, or `None` on the odd machine with no
    /// home directory. It is spelled out from the home directory rather than
    /// taken from `dirs::config_dir()`, which on macOS is
    /// `~/Library/Application Support`: this is a hand-editable dotfile and it
    /// belongs where a person would look for one.
    pub fn path() -> Option<PathBuf> {
        Some(
            dirs::home_dir()?
                .join(".config")
                .join(APP_DIR_NAME)
                .join("config.toml"),
        )
    }

    /// Read the file. A missing file (or no config directory) gives the
    /// defaults and no note; a file that does not parse gives the defaults and
    /// one line the popover can show, because refusing to start over a stray
    /// character would be worse than ignoring it.
    pub fn load() -> (Settings, Option<String>) {
        let Some(path) = Self::path() else {
            return (Settings::default(), None);
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Settings::default(), None)
            }
            Err(error) => {
                return (
                    Settings::default(),
                    Some(format!("Could not read config.toml: {error}")),
                )
            }
        };
        match toml::from_str::<Settings>(&text) {
            Ok(settings) => (settings.clamped(), None),
            Err(error) => (
                Settings::default(),
                Some(format!(
                    "config.toml ignored: {}",
                    first_line(&error.to_string())
                )),
            ),
        }
    }

    /// Write the file, creating `~/.config/scorebar` if it is not there yet.
    /// The error is a sentence rather than a type because its only consumer is
    /// the popover's notice line.
    ///
    /// Written to a temporary file and renamed over the target, which is
    /// atomic within a directory: a reader — this app on its next launch, or
    /// the user's editor — sees the whole old file or the whole new one,
    /// never a half-written one from a crash partway through.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "No config directory on this machine".to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        }
        let body = toml::to_string(self).map_err(|e| format!("Could not write settings: {e}"))?;
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, format!("{FILE_HEADER}{body}"))
            .map_err(|e| format!("Could not write {}: {e}", temp.display()))?;
        std::fs::rename(&temp, &path)
            .map_err(|e| format!("Could not write {}: {e}", path.display()))
    }

    /// The same settings pulled back into the range that works, so a
    /// hand-edited file cannot produce a refresh loop with no delay in it or a
    /// username made of spaces.
    fn clamped(mut self) -> Self {
        self.refresh_seconds = self.refresh_seconds.max(MIN_REFRESH_SECONDS);
        // `username = ""` is what a half-finished hand edit leaves behind, and
        // it has to read as "not set" rather than as a username Sleeper will
        // 404 on.
        self.sleeper_username = self
            .sleeper_username
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        self
    }

    /// The username, if there is a usable one. Trimmed, because
    /// [`Self::clamped`] trims on load and the Settings field may not have
    /// been through it yet.
    pub fn username(&self) -> Option<&str> {
        self.sleeper_username
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// Whether scorebar has enough to fetch anything at all.
    pub fn is_configured(&self) -> bool {
        self.username().is_some()
    }

    /// The poll interval as a duration, which is what the refresh loop wants.
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.refresh_seconds.max(MIN_REFRESH_SECONDS))
    }
}

/// TOML's parse errors are several lines with a caret diagram under them; the
/// notice line has room for the first one only.
fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or(message).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_a_minute_the_closest_game_and_no_username() {
        let settings = Settings::default();
        assert_eq!(settings.sleeper_username, None);
        assert_eq!(settings.menu_bar_title, MenuBarTitle::ClosestGame);
        assert_eq!(settings.refresh_seconds, 60);
        assert!(!settings.is_configured());
    }

    /// The default is the cdn's own cache window; anything faster is wasted.
    #[test]
    fn the_default_refresh_matches_the_cdn_cache_window() {
        assert_eq!(DEFAULT_REFRESH_SECONDS, 60);
        assert_eq!(
            Settings::default().refresh_interval(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn the_menu_bar_title_round_trips_through_its_string() {
        for title in MenuBarTitle::ALL {
            assert_eq!(MenuBarTitle::parse(title.as_config_str()), title);
        }
    }

    #[test]
    fn title_names_parse_whatever_their_case() {
        assert_eq!(MenuBarTitle::parse("Record"), MenuBarTitle::Record);
        assert_eq!(MenuBarTitle::parse(" GLYPH_ONLY "), MenuBarTitle::GlyphOnly);
    }

    /// A value from a newer build is not a reason to refuse the file.
    #[test]
    fn an_unknown_title_falls_back_to_the_default() {
        assert_eq!(MenuBarTitle::parse("margin"), MenuBarTitle::ClosestGame);
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let settings: Settings = toml::from_str("menu_bar_title = \"record\"\n").unwrap();
        assert_eq!(settings.menu_bar_title, MenuBarTitle::Record);
        assert_eq!(settings.refresh_seconds, 60);
        assert_eq!(settings.sleeper_username, None);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let settings: Settings =
            toml::from_str("menu_bar_title = \"record\"\nfuture_key = 3\n").unwrap();
        assert_eq!(settings.menu_bar_title, MenuBarTitle::Record);
    }

    #[test]
    fn a_written_file_reads_back_as_itself() {
        let settings = Settings {
            sleeper_username: Some("example_user".into()),
            menu_bar_title: MenuBarTitle::GlyphOnly,
            refresh_seconds: 120,
        };
        let text = toml::to_string(&settings).unwrap();
        assert!(text.contains("menu_bar_title = \"glyph_only\""));
        assert!(text.contains("sleeper_username = \"example_user\""));
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    /// An unset username has to survive the round trip as unset, not as `""`.
    #[test]
    fn a_missing_username_round_trips_as_missing() {
        let settings = Settings::default();
        let text = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn too_fast_a_refresh_is_clamped_to_the_floor() {
        let settings = Settings {
            refresh_seconds: 0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(settings.refresh_seconds, MIN_REFRESH_SECONDS);
        assert_eq!(
            settings.refresh_interval(),
            Duration::from_secs(MIN_REFRESH_SECONDS)
        );

        // A slower refresh than the default is the user's business.
        let slow = Settings {
            refresh_seconds: 600,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(slow.refresh_seconds, 600);
    }

    /// A username of spaces is a half-finished edit, not a username.
    #[test]
    fn a_blank_username_reads_as_no_username() {
        let settings = Settings {
            sleeper_username: Some("   ".into()),
            ..Settings::default()
        }
        .clamped();
        assert_eq!(settings.sleeper_username, None);
        assert!(!settings.is_configured());

        let padded = Settings {
            sleeper_username: Some("  example_user \n".into()),
            ..Settings::default()
        };
        assert_eq!(padded.username(), Some("example_user"));
        assert_eq!(
            padded.clamped().sleeper_username.as_deref(),
            Some("example_user")
        );
    }

    #[test]
    fn the_config_and_cache_directories_share_one_name() {
        assert_eq!(APP_DIR_NAME, "scorebar");
        if let Some(path) = Settings::path() {
            assert!(path.ends_with("scorebar/config.toml"));
        }
    }

    #[test]
    fn a_malformed_file_becomes_one_line() {
        let error = toml::from_str::<Settings>("menu_bar_title = ").unwrap_err();
        let note = first_line(&error.to_string());
        assert!(!note.is_empty());
        assert!(!note.contains('\n'));
    }

    /// An empty string is a *broken* value, not an unknown one: it comes from
    /// `menu_bar_title = ""`, which almost certainly means the user deleted
    /// half a line, and saying so is more useful than silently defaulting.
    #[test]
    fn an_empty_title_is_rejected_rather_than_defaulted() {
        assert!(toml::from_str::<Settings>("menu_bar_title = \"\"\n").is_err());
    }
}
