//! What can go wrong building a snapshot.
//!
//! Almost everything is Sleeper's to fail, so most of this enum is a wrapper.
//! The two cases core adds are the ones that are not failures of the api at
//! all — the api answered correctly, and the correct answer was "no". An
//! unknown username comes back as HTTP 200 with a `null` body, and an account
//! with no leagues comes back as HTTP 200 with `[]`. Both are things the user
//! typed wrong or a season that has not rolled over yet, and both need a
//! sentence the popover can print at somebody rather than a status code.

use thiserror::Error;

/// Anything that can fail between a username and a [`crate::Snapshot`].
#[derive(Debug, Error)]
pub enum Error {
    /// The api call itself failed, or its payload did not decode.
    #[error(transparent)]
    Sleeper(#[from] sleeper::Error),

    /// Sleeper has no account by that name. Distinct from a transport failure
    /// and from an empty league list, because it is the one of the three the
    /// user can fix by retyping.
    #[error("no sleeper account named \"{username}\"")]
    UnknownUsername {
        /// What was typed, quoted back so the message names it.
        username: String,
    },

    /// The account exists and is in no leagues this season. Normal in the
    /// offseason before Sleeper rolls dynasty leagues over, and normal for an
    /// account that has never played.
    #[error("that account is not in any leagues for the {season} season")]
    NoLeagues {
        /// The season asked for.
        season: u16,
    },
}

impl Error {
    /// Whether this is worth trying again shortly.
    ///
    /// The refresh loop backs off on a transient failure and keeps showing the
    /// cached snapshot; on anything else it stops and says why, because
    /// retrying a misspelled username every five minutes forever is not a
    /// recovery strategy.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Sleeper(error) => error.is_transient(),
            Error::UnknownUsername { .. } | Error::NoLeagues { .. } => false,
        }
    }
}

/// The crate's result alias, shadowing [`std::result::Result`] the same way
/// the sleeper crate's does.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_cases_core_adds_read_as_sentences() {
        let unknown = Error::UnknownUsername {
            username: "nobody".to_owned(),
        };
        assert_eq!(unknown.to_string(), "no sleeper account named \"nobody\"");

        let empty = Error::NoLeagues { season: 2026 };
        assert_eq!(
            empty.to_string(),
            "that account is not in any leagues for the 2026 season"
        );
    }

    /// A wrapped sleeper error should not gain a prefix on the way through —
    /// `#[error(transparent)]` is what keeps "sleeper answered 503 for ..."
    /// from becoming "sleeper error: sleeper answered 503 for ...".
    #[test]
    fn a_wrapped_sleeper_error_keeps_its_own_words() {
        let error = Error::from(sleeper::Error::Status {
            code: 503,
            endpoint: "/v1/state/nfl".to_owned(),
        });
        assert_eq!(error.to_string(), "sleeper answered 503 for /v1/state/nfl");
    }

    #[test]
    fn only_sleeper_failures_are_worth_retrying() {
        assert!(Error::from(sleeper::Error::Status {
            code: 503,
            endpoint: "/v1/state/nfl".to_owned(),
        })
        .is_transient());
        assert!(!Error::from(sleeper::Error::NotFound {
            what: "user \"nobody\"".to_owned(),
        })
        .is_transient());
        assert!(!Error::UnknownUsername {
            username: "nobody".to_owned(),
        }
        .is_transient());
        assert!(!Error::NoLeagues { season: 2026 }.is_transient());
    }
}
