//! A DLP finding allowed on purpose: one value or one detector, for one sandbox
//! or the whole environment, until a given moment.
//!
//! Matching is by the salted fingerprint, never by the value: this type cannot
//! carry content even if a caller wanted it to. Specificity is a number, so the
//! narrowest exception always wins and a later broad one cannot silently undo a
//! narrow one.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::privacy::{Action, Detector};

/// A finding allowed on purpose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PrivacyException {
    pub id: Uuid,
    pub environment_id: Uuid,
    /// `None` is the environment's exception, not a bug.
    pub sandbox_id: Option<Uuid>,
    pub detector: Detector,
    /// `None` means every value of this detector.
    pub value_fingerprint: Option<String>,
    /// `None` means every destination.
    pub destination: Option<String>,
    /// `Allow`, `Replace` or `Pseudonymize`; never `Block`.
    pub action: Action,
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// One finding as the matcher sees it.
pub struct FindingKey<'a> {
    pub sandbox_id: Uuid,
    pub detector: Detector,
    pub fingerprint: &'a str,
    pub destination: Option<&'a str>,
}

/// The most specific exception that covers a finding.
///
/// A sandbox row beats an environment row, an exact fingerprint beats "any value
/// of this detector", and a named destination beats "any destination". A tie goes
/// to the newest, so a later decision replaces an earlier one of equal reach.
pub fn best_match<'a>(
    exceptions: &'a [PrivacyException],
    key: &FindingKey<'_>,
    now: DateTime<Utc>,
) -> Option<&'a PrivacyException> {
    let host = key
        .destination
        .map(|value| value.split(':').next().unwrap_or(value).to_ascii_lowercase());
    let mut best: Option<(u8, &PrivacyException)> = None;

    for exception in exceptions {
        if exception.detector != key.detector {
            continue;
        }
        if exception.expires_at.is_some_and(|expires| expires <= now) {
            continue;
        }
        if exception.sandbox_id.is_some_and(|id| id != key.sandbox_id) {
            continue;
        }
        if exception
            .value_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint != key.fingerprint)
        {
            continue;
        }
        if let Some(named) = exception.destination.as_deref() {
            let named = named.to_ascii_lowercase();
            if host.as_deref() != Some(named.as_str()) {
                continue;
            }
        }

        let mut score = 0u8;
        if exception.sandbox_id.is_some() {
            score += 4;
        }
        if exception.value_fingerprint.is_some() {
            score += 2;
        }
        if exception.destination.is_some() {
            score += 1;
        }
        let better = match best {
            None => true,
            Some((best_score, current)) => {
                score > best_score
                    || (score == best_score && exception.created_at > current.created_at)
            }
        };
        if better {
            best = Some((score, exception));
        }
    }

    best.map(|(_, exception)| exception)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exception(
        sandbox: Option<Uuid>,
        fingerprint: Option<&str>,
        destination: Option<&str>,
        created_seconds: i64,
    ) -> PrivacyException {
        let base = Utc::now();
        PrivacyException {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            sandbox_id: sandbox,
            detector: Detector::Iban,
            value_fingerprint: fingerprint.map(str::to_string),
            destination: destination.map(str::to_string),
            action: Action::Allow,
            expires_at: None,
            reason: String::new(),
            created_by: None,
            created_at: base + chrono::Duration::seconds(created_seconds),
        }
    }

    const FINGERPRINT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn an_exact_value_beats_the_whole_detector() {
        let sandbox = Uuid::new_v4();
        let exceptions = vec![
            exception(Some(sandbox), None, None, 1),
            exception(Some(sandbox), Some(FINGERPRINT), None, 0),
        ];
        let key = FindingKey {
            sandbox_id: sandbox,
            detector: Detector::Iban,
            fingerprint: FINGERPRINT,
            destination: None,
        };
        let found = best_match(&exceptions, &key, Utc::now()).expect("a match");
        assert_eq!(found.value_fingerprint.as_deref(), Some(FINGERPRINT));
    }

    #[test]
    fn a_sandbox_beats_the_environment() {
        let sandbox = Uuid::new_v4();
        let exceptions = vec![
            exception(None, Some(FINGERPRINT), None, 5),
            exception(Some(sandbox), Some(FINGERPRINT), None, 0),
        ];
        let key = FindingKey {
            sandbox_id: sandbox,
            detector: Detector::Iban,
            fingerprint: FINGERPRINT,
            destination: None,
        };
        let found = best_match(&exceptions, &key, Utc::now()).expect("a match");
        assert_eq!(found.sandbox_id, Some(sandbox));
    }

    #[test]
    fn an_empty_list_has_no_match() {
        let key = FindingKey {
            sandbox_id: Uuid::new_v4(),
            detector: Detector::Iban,
            fingerprint: FINGERPRINT,
            destination: None,
        };
        assert!(best_match(&[], &key, Utc::now()).is_none());
    }

    #[test]
    fn a_named_destination_must_ignore_the_port_and_the_case() {
        let sandbox = Uuid::new_v4();
        let named = exception(Some(sandbox), None, Some("API.Example.com"), 0);
        let key = FindingKey {
            sandbox_id: sandbox,
            detector: Detector::Iban,
            fingerprint: FINGERPRINT,
            destination: Some("api.example.com:443"),
        };
        assert!(best_match(&[named], &key, Utc::now()).is_some());
    }

    #[test]
    fn an_expired_exception_does_not_count() {
        let sandbox = Uuid::new_v4();
        let mut expired = exception(Some(sandbox), Some(FINGERPRINT), None, 0);
        expired.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        let key = FindingKey {
            sandbox_id: sandbox,
            detector: Detector::Iban,
            fingerprint: FINGERPRINT,
            destination: None,
        };
        assert!(best_match(&[expired], &key, Utc::now()).is_none());
    }

    #[test]
    fn another_value_of_the_same_detector_does_not_match() {
        let sandbox = Uuid::new_v4();
        let exceptions = vec![exception(Some(sandbox), Some(FINGERPRINT), None, 0)];
        let key = FindingKey {
            sandbox_id: sandbox,
            detector: Detector::Iban,
            fingerprint: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            destination: None,
        };
        assert!(best_match(&exceptions, &key, Utc::now()).is_none());
    }
}
