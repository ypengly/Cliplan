pub mod identity;

use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use identity::generate_code;

/// A short-lived pairing session. The QR code encodes a URL containing
/// `session_id`; the 6-digit `code` is shown only on the server's own
/// terminal (or dashboard, if viewed on the host itself) and must be typed
/// in by the connecting device. This means physical/local access to the
/// host is required to complete a pairing -- scanning the QR alone is not
/// sufficient, which is the point of section 14 ("passwordless" is not
/// "unauthenticated").
#[derive(Debug, Clone)]
pub struct PairingSession {
    pub session_id: String,
    pub code: String,
    pub expires_at: Instant,
}

impl PairingSession {
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

const MAX_FAILED_ATTEMPTS: u32 = 5;

pub struct PairingManager {
    session: Mutex<Option<PairingSession>>,
    failed_attempts: Mutex<u32>,
    ttl: Duration,
}

impl PairingManager {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            failed_attempts: Mutex::new(0),
            ttl: Duration::from_secs(5 * 60),
        }
    }

    /// Start (or restart) a pairing session and return it.
    pub fn start_session(&self) -> PairingSession {
        let session = PairingSession {
            session_id: Uuid::new_v4().to_string(),
            code: generate_code(),
            expires_at: Instant::now() + self.ttl,
        };
        *self.session.lock().unwrap() = Some(session.clone());
        *self.failed_attempts.lock().unwrap() = 0;
        session
    }

    /// Return the current session if one is active and not expired.
    pub fn current(&self) -> Option<PairingSession> {
        let mut guard = self.session.lock().unwrap();
        if let Some(s) = guard.as_ref() {
            if s.is_expired() {
                *guard = None;
                return None;
            }
        }
        guard.clone()
    }

    /// Validate a `(session_id, code)` pair against the active session.
    /// After too many wrong-code attempts the session is invalidated
    /// outright, so a 6-digit code cannot be brute-forced by repeated
    /// guessing within its 5-minute lifetime (section 14/22).
    pub fn validate(&self, session_id: &str, code: &str) -> bool {
        let ok = match self.current() {
            Some(s) => {
                identity::secure_compare(&s.session_id, session_id)
                    && identity::secure_compare(&s.code, code)
            }
            None => false,
        };

        if !ok {
            let mut attempts = self.failed_attempts.lock().unwrap();
            *attempts += 1;
            if *attempts >= MAX_FAILED_ATTEMPTS {
                *self.session.lock().unwrap() = None;
            }
        }

        ok
    }

    /// Same as `validate`, but for the manual entry-only flow (section 14)
    /// where the connecting device never saw a QR-encoded session id --
    /// just the code shown on the host's screen. Since only one session is
    /// ever active at a time, the code alone is enough to identify it.
    pub fn validate_code(&self, code: &str) -> bool {
        let ok = match self.current() {
            Some(s) => identity::secure_compare(&s.code, code),
            None => false,
        };

        if !ok {
            let mut attempts = self.failed_attempts.lock().unwrap();
            *attempts += 1;
            if *attempts >= MAX_FAILED_ATTEMPTS {
                *self.session.lock().unwrap() = None;
            }
        }

        ok
    }
}

impl Default for PairingManager {
    fn default() -> Self {
        Self::new()
    }
}
