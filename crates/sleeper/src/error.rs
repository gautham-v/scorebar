//! What can go wrong talking to Sleeper.
//!
//! The one non-obvious case is not-found. Sleeper answers an unknown league
//! with HTTP 404 and a `null` body, but an unknown username with HTTP **200**
//! and a `null` body, and an unknown user's league list with 200 and `[]`. A
//! success status therefore does not mean there is a payload, so the client
//! decodes into `Option<T>` and turns both shapes into the same
//! [`Error::NotFound`] — callers should never have to know which endpoint
//! chose which convention.

use thiserror::Error;

/// Anything that can fail between asking Sleeper a question and getting a
/// typed answer back.
#[derive(Debug, Error)]
pub enum Error {
    /// The request never produced a response: DNS, TLS, connect, timeout.
    #[error("sleeper request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// A response arrived but did not match the types in [`crate::model`].
    /// Carries the endpoint because the payloads are large and the line and
    /// column in the serde error are useless without knowing which one.
    #[error("could not decode {endpoint}: {source}")]
    Decode {
        /// The request path, e.g. `/v1/league/<id>/matchups/2`.
        endpoint: String,
        /// The underlying serde failure.
        source: serde_json::Error,
    },

    /// The thing asked for does not exist. Both a 404 and a 200 with a `null`
    /// body land here.
    #[error("sleeper has no {what}")]
    NotFound {
        /// What was asked for, phrased for a message: `user "someone"`.
        what: String,
    },

    /// Any other non-success status. 429 and 5xx show up here, which is the
    /// signal to back off rather than retry immediately.
    #[error("sleeper answered {code} for {endpoint}")]
    Status {
        /// The HTTP status code.
        code: u16,
        /// The request path that produced it.
        endpoint: String,
    },
}

impl Error {
    /// Whether this is worth trying again shortly. Rate limits and server
    /// errors are; a 404 or a decode failure will fail the same way forever.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Http(error) => error.is_timeout() || error.is_connect(),
            Error::Status { code, .. } => *code == 429 || *code >= 500,
            Error::Decode { .. } | Error::NotFound { .. } => false,
        }
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_the_ones_worth_retrying() {
        assert!(Error::Status {
            code: 429,
            endpoint: "/v1/state/nfl".into()
        }
        .is_transient());
        assert!(Error::Status {
            code: 503,
            endpoint: "/v1/state/nfl".into()
        }
        .is_transient());
        assert!(!Error::Status {
            code: 400,
            endpoint: "/v1/state/nfl".into()
        }
        .is_transient());
        assert!(!Error::NotFound {
            what: "user \"nobody\"".into()
        }
        .is_transient());
    }

    #[test]
    fn not_found_reads_as_a_sentence() {
        let error = Error::NotFound {
            what: "user \"nobody\"".into(),
        };
        assert_eq!(error.to_string(), "sleeper has no user \"nobody\"");
    }
}
