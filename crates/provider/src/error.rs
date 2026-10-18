use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    Unsupported {
        operation: &'static str,
    },
    InvalidInput {
        field: &'static str,
    },
    NotFound {
        entity: &'static str,
    },
    SecretAccessDenied,
    AuthorizationRequired,
    OwnerAppUnavailable,
    AppConfigurationFailed {
        operation: &'static str,
        upstream_code: Option<i64>,
        status: Option<u16>,
    },
    Upstream {
        operation: &'static str,
        upstream_code: Option<i64>,
        status: Option<u16>,
        retryable: bool,
    },
    EventSinkClosed,
    DuplicateRegistration {
        id: String,
    },
    UnknownRegistration {
        id: String,
    },
    InvalidSchema {
        location: &'static str,
    },
    SchemaMismatch {
        location: &'static str,
    },
    Timeout {
        operation: &'static str,
    },
    External {
        operation: &'static str,
        retryable: bool,
    },
}

impl ProviderError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unsupported { .. } => "unsupported",
            Self::InvalidInput { .. } => "invalid_input",
            Self::NotFound { .. } => "not_found",
            Self::SecretAccessDenied => "secret_access_denied",
            Self::AuthorizationRequired => "authorization_required",
            Self::OwnerAppUnavailable => "owner_app_unavailable",
            Self::AppConfigurationFailed { .. } => "app_configuration_failed",
            Self::Upstream { .. } => "external",
            Self::EventSinkClosed => "event_sink_closed",
            Self::DuplicateRegistration { .. } => "duplicate_registration",
            Self::UnknownRegistration { .. } => "unknown_registration",
            Self::InvalidSchema { .. } => "invalid_schema",
            Self::SchemaMismatch { .. } => "schema_mismatch",
            Self::Timeout { .. } => "timeout",
            Self::External { .. } => "external",
        }
    }

    /// Only trusted operation identifiers and numeric upstream metadata may be logged.
    pub const fn upstream_diagnostic(&self) -> Option<(&'static str, Option<i64>, Option<u16>)> {
        match self {
            Self::AppConfigurationFailed {
                operation,
                upstream_code,
                status,
            }
            | Self::Upstream {
                operation,
                upstream_code,
                status,
                ..
            } => Some((operation, *upstream_code, *status)),
            _ => None,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for ProviderError {}

/// A secret can only be exposed by an explicit Provider action and is always redacted in logs.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);
impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
