use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};

/// Standard Matrix error response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixError {
    pub errcode: ErrorCode,
    pub error: String,
    #[serde(skip)]
    pub status: StatusCode,
}

impl MatrixError {
    pub fn new(status: StatusCode, errcode: ErrorCode, error: impl Into<String>) -> Self {
        Self {
            errcode,
            error: error.into(),
            status,
        }
    }

    // -- Convenience constructors for common errors --

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, msg)
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, msg)
    }

    pub fn unknown(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Unknown, msg)
    }

    pub fn bad_json(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadJson, msg)
    }

    pub fn not_json() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::NotJson,
            "Content-Type must be application/json",
        )
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::UnknownToken, msg)
    }

    pub fn missing_token() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::MissingToken,
            "Missing access token",
        )
    }

    pub fn limit_exceeded(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, ErrorCode::LimitExceeded, msg)
    }

    pub fn unrecognized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Unrecognized, msg)
    }

    pub fn user_in_use() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::UserInUse,
            "User ID already taken",
        )
    }

    pub fn invalid_username(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidUsername, msg)
    }

    pub fn exclusive(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Exclusive, msg)
    }
}

impl IntoResponse for MatrixError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = serde_json::to_string(&self).unwrap_or_else(|_| {
            r#"{"errcode":"M_UNKNOWN","error":"Failed to serialize error"}"#.to_string()
        });
        (status, [("content-type", "application/json")], body).into_response()
    }
}

impl std::fmt::Display for MatrixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.errcode, self.error)
    }
}

impl std::error::Error for MatrixError {}

/// All standard Matrix error codes.
///
/// Serialized as `M_UPPER_SNAKE_CASE` per the Matrix specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "M_FORBIDDEN")]
    Forbidden,
    #[serde(rename = "M_UNKNOWN_TOKEN")]
    UnknownToken,
    #[serde(rename = "M_MISSING_TOKEN")]
    MissingToken,
    #[serde(rename = "M_BAD_JSON")]
    BadJson,
    #[serde(rename = "M_NOT_JSON")]
    NotJson,
    #[serde(rename = "M_NOT_FOUND")]
    NotFound,
    #[serde(rename = "M_LIMIT_EXCEEDED")]
    LimitExceeded,
    #[serde(rename = "M_UNKNOWN")]
    Unknown,
    #[serde(rename = "M_UNRECOGNIZED")]
    Unrecognized,
    #[serde(rename = "M_UNAUTHORIZED")]
    Unauthorized,
    #[serde(rename = "M_USER_DEACTIVATED")]
    UserDeactivated,
    #[serde(rename = "M_USER_IN_USE")]
    UserInUse,
    #[serde(rename = "M_INVALID_USERNAME")]
    InvalidUsername,
    #[serde(rename = "M_ROOM_IN_USE")]
    RoomInUse,
    #[serde(rename = "M_INVALID_ROOM_STATE")]
    InvalidRoomState,
    #[serde(rename = "M_THREEPID_IN_USE")]
    ThreepidInUse,
    #[serde(rename = "M_THREEPID_NOT_FOUND")]
    ThreepidNotFound,
    #[serde(rename = "M_THREEPID_AUTH_FAILED")]
    ThreepidAuthFailed,
    #[serde(rename = "M_THREEPID_DENIED")]
    ThreepidDenied,
    #[serde(rename = "M_SERVER_NOT_TRUSTED")]
    ServerNotTrusted,
    #[serde(rename = "M_UNSUPPORTED_ROOM_VERSION")]
    UnsupportedRoomVersion,
    #[serde(rename = "M_INCOMPATIBLE_ROOM_VERSION")]
    IncompatibleRoomVersion,
    #[serde(rename = "M_BAD_STATE")]
    BadState,
    #[serde(rename = "M_GUEST_ACCESS_FORBIDDEN")]
    GuestAccessForbidden,
    #[serde(rename = "M_CAPTCHA_NEEDED")]
    CaptchaNeeded,
    #[serde(rename = "M_CAPTCHA_INVALID")]
    CaptchaInvalid,
    #[serde(rename = "M_MISSING_PARAM")]
    MissingParam,
    #[serde(rename = "M_INVALID_PARAM")]
    InvalidParam,
    #[serde(rename = "M_TOO_LARGE")]
    TooLarge,
    #[serde(rename = "M_EXCLUSIVE")]
    Exclusive,
    #[serde(rename = "M_RESOURCE_LIMIT_EXCEEDED")]
    ResourceLimitExceeded,
    #[serde(rename = "M_CANNOT_LEAVE_SERVER_NOTICE_ROOM")]
    CannotLeaveServerNoticeRoom,
    #[serde(rename = "M_WEAK_PASSWORD")]
    WeakPassword,
    #[serde(rename = "M_UNABLE_TO_AUTHORISE_JOIN")]
    UnableToAuthoriseJoin,
    #[serde(rename = "M_UNABLE_TO_GRANT_JOIN")]
    UnableToGrantJoin,
    #[serde(rename = "M_DUPLICATE_ANNOTATION")]
    DuplicateAnnotation,
    #[serde(rename = "M_NOT_YET_UPLOADED")]
    NotYetUploaded,
    #[serde(rename = "M_CANNOT_OVERWRITE_MEDIA")]
    CannotOverwriteMedia,
    #[serde(rename = "M_UNKNOWN_POS")]
    UnknownPos,
    #[serde(rename = "M_URL_NOT_SET")]
    UrlNotSet,
    #[serde(rename = "M_BAD_ALIAS")]
    BadAlias,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "M_UNKNOWN".to_string());
        write!(f, "{s}")
    }
}
