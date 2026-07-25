use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestFailure {
    Timeout,
    Connect,
    Redirect,
    Body,
    Other,
}

impl std::fmt::Display for RequestFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Timeout => "request timed out",
            Self::Connect => "connection failed",
            Self::Redirect => "redirect failed",
            Self::Body => "response body failed",
            Self::Other => "request failed",
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiError {
    #[error("controller URL is invalid")]
    InvalidControllerUrl,

    #[error("controller URL must use http or https")]
    UnsupportedControllerScheme,

    #[error("controller URL must not contain credentials, a query, or a fragment")]
    UnsafeControllerUrl,

    #[error("failed to initialize the HTTP client")]
    ClientInitialization,

    #[error("failed to construct the {operation} endpoint")]
    InvalidEndpoint { operation: &'static str },

    #[error("{operation}: {kind}")]
    Request {
        operation: &'static str,
        kind: RequestFailure,
    },

    #[error("{operation} returned HTTP {status}")]
    UnexpectedStatus {
        operation: &'static str,
        status: u16,
    },

    #[error("{operation} response exceeded {limit_bytes} bytes")]
    ResponseTooLarge {
        operation: &'static str,
        limit_bytes: usize,
    },

    #[error("{operation} returned an invalid JSON response")]
    InvalidResponse { operation: &'static str },
}
