use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("invalid proxy configuration: {0}")]
    BadProxy(String),
    #[error("all mirrors failed: {0}")]
    AllMirrorsFailed(String),
    #[error("http status {0}")]
    Status(u16),
    #[error("integrity check failed: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("invalid download url: {0}")]
    InvalidUrl(String),
    #[error("download exceeds the size limit")]
    TooLarge,
    #[error("unexpected server response: {0}")]
    BadResponse(String),
    #[error("download paused")]
    Paused,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
}

impl NetError {
    /// Transient failures worth retrying (same or next mirror).
    pub fn is_retryable(&self) -> bool {
        match self {
            NetError::Status(s) => *s == 408 || *s == 429 || *s >= 500,
            NetError::Request(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_decode(),
            NetError::Io(_) => true,
            // A misbehaving server or a stale partial file: start over, possibly on another mirror.
            NetError::BadResponse(_) => true,
            _ => false,
        }
    }

    /// Short hint shown in the UI next to the error.
    pub fn hint(&self) -> &'static str {
        match self {
            NetError::AllMirrorsFailed(_) | NetError::Request(_) | NetError::Status(_) => {
                "check_connection_proxy_or_mirror"
            }
            NetError::HashMismatch { .. } => "file_corrupted_retry",
            _ => "",
        }
    }
}

impl Serialize for NetError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
