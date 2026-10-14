//! Typed errors shared by HTTP, local Agent calls, and business APIs.
pub use crate::management::ErrorCode as Code;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    code: Code,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}
impl Status {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }
    pub fn code(&self) -> Code {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(Code::Cancelled, message)
    }
    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(Code::Unknown, message)
    }
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidArgument, message)
    }
    pub fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self::new(Code::DeadlineExceeded, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Code::NotFound, message)
    }
    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(Code::AlreadyExists, message)
    }
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(Code::PermissionDenied, message)
    }
    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(Code::ResourceExhausted, message)
    }
    pub fn failed_precondition(message: impl Into<String>) -> Self {
        Self::new(Code::FailedPrecondition, message)
    }
    pub fn aborted(message: impl Into<String>) -> Self {
        Self::new(Code::Aborted, message)
    }
    pub fn out_of_range(message: impl Into<String>) -> Self {
        Self::new(Code::OutOfRange, message)
    }
    pub fn unimplemented(message: impl Into<String>) -> Self {
        Self::new(Code::Unimplemented, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Code::Internal, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Code::Unavailable, message)
    }
    pub fn data_loss(message: impl Into<String>) -> Self {
        Self::new(Code::DataLoss, message)
    }
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(Code::Unauthenticated, message)
    }
    pub fn http_status(&self) -> hyper::StatusCode {
        use hyper::StatusCode as H;
        match self.code {
            Code::Unauthenticated => H::UNAUTHORIZED,
            Code::PermissionDenied => H::FORBIDDEN,
            Code::NotFound => H::NOT_FOUND,
            Code::AlreadyExists => H::CONFLICT,
            Code::Unavailable => H::SERVICE_UNAVAILABLE,
            Code::ResourceExhausted => H::PAYLOAD_TOO_LARGE,
            Code::Internal | Code::DataLoss | Code::Unknown => H::INTERNAL_SERVER_ERROR,
            _ => H::BAD_REQUEST,
        }
    }
}
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "code: '{}'", self.code)?;
        if !self.message.is_empty() {
            write!(f, ", message: {:?}", self.message)?;
        }
        Ok(())
    }
}
impl std::error::Error for Status {}
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ok => "The operation completed successfully",
            Self::Cancelled => "The operation was cancelled",
            Self::Unknown => "Unknown error",
            Self::InvalidArgument => "Client specified an invalid argument",
            Self::DeadlineExceeded => "Deadline expired before operation could complete",
            Self::NotFound => "Some requested entity was not found",
            Self::AlreadyExists => "Some entity that we attempted to create already exists",
            Self::PermissionDenied => {
                "The caller does not have permission to execute the specified operation"
            }
            Self::ResourceExhausted => "Some resource has been exhausted",
            Self::FailedPrecondition => {
                "The system is not in a state required for the operation's execution"
            }
            Self::Aborted => "The operation was aborted",
            Self::OutOfRange => "Operation was attempted past the valid range",
            Self::Unimplemented => "Operation is not implemented or not supported",
            Self::Internal => "Internal error",
            Self::Unavailable => "The service is currently unavailable",
            Self::DataLoss => "Unrecoverable data loss or corruption",
            Self::Unauthenticated => "The request does not have valid authentication credentials",
        })
    }
}
impl Code {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Ok,
            1 => Self::Cancelled,
            2 => Self::Unknown,
            3 => Self::InvalidArgument,
            4 => Self::DeadlineExceeded,
            5 => Self::NotFound,
            6 => Self::AlreadyExists,
            7 => Self::PermissionDenied,
            8 => Self::ResourceExhausted,
            9 => Self::FailedPrecondition,
            10 => Self::Aborted,
            11 => Self::OutOfRange,
            12 => Self::Unimplemented,
            13 => Self::Internal,
            14 => Self::Unavailable,
            15 => Self::DataLoss,
            16 => Self::Unauthenticated,
            _ => Self::Unknown,
        }
    }
}
/// A validated service input.
pub struct Request<T>(T);
impl<T> Request<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
    pub fn get_ref(&self) -> &T {
        &self.0
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}
/// A service result; keeps streaming and ordinary replies uniform.
#[derive(Debug)]
pub struct Response<T>(T);
impl<T> Response<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}
