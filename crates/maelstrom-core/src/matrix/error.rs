use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};

/// The standard Matrix JSON error response body
/// ([spec](https://spec.matrix.org/v1.13/client-server-api/#standard-error-response)).
///
/// Every failed Matrix API call returns a JSON object with at least two
/// fields: `errcode` (a machine-readable `M_*` error code) and `error`
/// (a human-readable description). This struct models that object and
/// carries an HTTP status code that is used when converting into an Axum
/// response but is **not** serialized into the JSON body.
///
/// # How it fits into handlers
///
/// `MatrixError` implements [`IntoResponse`], so any handler can return
/// `Result<Json<T>, MatrixError>` and Axum will do the right thing:
///
/// ```rust,ignore
/// async fn get_room(room_id: &str) -> Result<Json<Room>, MatrixError> {
///     let room = store.get(room_id)
///         .ok_or_else(|| MatrixError::not_found("Room not found"))?;
///     Ok(Json(room))
/// }
/// ```
///
/// # JSON shape produced
///
/// ```json
/// {
///   "errcode": "M_NOT_FOUND",
///   "error": "Room not found"
/// }
/// ```
///
/// The HTTP status code (e.g. 404) is set on the response but is **not**
/// included in the JSON body — that's per spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixError {
    /// The machine-readable error code (e.g. `M_FORBIDDEN`).
    pub errcode: ErrorCode,
    /// A human-readable description of the error, intended for developers
    /// rather than end users.
    pub error: String,
    /// The HTTP status code to return. Skipped during JSON serialization
    /// because the spec says it goes on the HTTP response, not in the body.
    #[serde(skip)]
    pub status: StatusCode,
}

impl MatrixError {
    /// Build a `MatrixError` from its raw parts. Prefer the convenience
    /// constructors below unless you need a non-standard combination.
    pub fn new(status: StatusCode, errcode: ErrorCode, error: impl Into<String>) -> Self {
        Self {
            errcode,
            error: error.into(),
            status,
        }
    }

    // -- Convenience constructors for common errors --
    //
    // Each one pairs the correct HTTP status with the right `M_*` code so
    // callers never have to remember the mapping themselves.

    /// **404 / M_NOT_FOUND** — the requested resource does not exist.
    ///
    /// Use for missing rooms, events, users, aliases, or any entity lookup
    /// that came up empty.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, msg)
    }

    /// **403 / M_FORBIDDEN** — the user is authenticated but lacks permission.
    ///
    /// Typical causes: trying to send in a room you're not joined to,
    /// banning someone when you're not a moderator, accessing an admin
    /// endpoint as a regular user.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, msg)
    }

    /// **500 / M_UNKNOWN** — catch-all for unexpected internal errors.
    ///
    /// Use sparingly. If the failure has a more specific code (e.g.
    /// `M_NOT_FOUND`, `M_BAD_JSON`), use that instead. This is the
    /// "something went wrong and we don't have a better code" fallback.
    pub fn unknown(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Unknown, msg)
    }

    /// **400 / M_BAD_JSON** — the request body is JSON but doesn't match
    /// the expected schema.
    ///
    /// Use when deserialization succeeds (it's valid JSON) but a required
    /// field is missing, a value is out of range, etc.
    pub fn bad_json(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadJson, msg)
    }

    /// **400 / M_NOT_JSON** — the request body is not valid JSON at all.
    ///
    /// Returned when the `Content-Type` header is wrong or the body
    /// fails to parse as JSON. Takes no message because the situation is
    /// always the same.
    pub fn not_json() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::NotJson,
            "Content-Type must be application/json",
        )
    }

    /// **401 / M_UNKNOWN_TOKEN** — the access token is present but invalid
    /// or expired.
    ///
    /// The spec uses `M_UNKNOWN_TOKEN` (not `M_UNAUTHORIZED`) for this
    /// case. The client should prompt the user to log in again.
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::UnknownToken, msg)
    }

    /// **401 / M_MISSING_TOKEN** — no access token was provided at all.
    ///
    /// Different from `unauthorized`: here the `Authorization` header (or
    /// `access_token` query param) is entirely absent.
    pub fn missing_token() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::MissingToken,
            "Missing access token",
        )
    }

    /// **429 / M_LIMIT_EXCEEDED** — rate limit hit.
    ///
    /// The spec allows an optional `retry_after_ms` field; if you need
    /// that, build with `MatrixError::new` and add the field manually.
    pub fn limit_exceeded(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, ErrorCode::LimitExceeded, msg)
    }

    /// **400 / M_UNRECOGNIZED** — the endpoint exists but the request
    /// contains unrecognized parameters or query arguments.
    ///
    /// Not to be confused with a plain 404 for unknown endpoints.
    pub fn unrecognized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Unrecognized, msg)
    }

    /// **400 / M_USER_IN_USE** — registration failed because the requested
    /// username is already taken.
    pub fn user_in_use() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::UserInUse,
            "User ID already taken",
        )
    }

    /// **400 / M_INVALID_USERNAME** — the requested username contains
    /// disallowed characters or is otherwise invalid.
    pub fn invalid_username(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidUsername, msg)
    }

    /// **400 / M_EXCLUSIVE** — the requested resource (usually an alias or
    /// user namespace) is reserved by an application service.
    pub fn exclusive(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Exclusive, msg)
    }

    /// **413 / M_TOO_LARGE** — the request payload exceeds the server's
    /// size limit (most commonly hit on media uploads).
    pub fn too_large(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge, msg)
    }

    /// **400 / M_BAD_ALIAS** — a room alias in the request is malformed or
    /// points to a room that doesn't exist.
    pub fn bad_alias(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadAlias, msg)
    }
}

/// Converts the error into an Axum HTTP response with the correct status
/// code and a `Content-Type: application/json` body containing `errcode`
/// and `error`. This is what makes `Result<T, MatrixError>` work as a
/// handler return type.
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

/// All standard Matrix error codes (the `errcode` field in error responses).
///
/// The Matrix spec defines these with an `M_` prefix and `UPPER_SNAKE_CASE`
/// naming. Serde's `rename` attributes handle the conversion so Rust code
/// uses idiomatic `PascalCase` variants while the wire format stays
/// spec-compliant.
///
/// # Most common codes you'll encounter day-to-day
///
/// | Code | When you'll see it |
/// |------|--------------------|
/// | `Forbidden` | User lacks permission for the action |
/// | `NotFound` | Room, event, user, or alias doesn't exist |
/// | `UnknownToken` | Access token is expired or invalid |
/// | `MissingToken` | No access token provided at all |
/// | `BadJson` | Request body is valid JSON but wrong shape |
/// | `LimitExceeded` | Rate limiter kicked in |
/// | `Unknown` | Catch-all internal error |
///
/// The full list below covers the entire spec as of v1.13. You won't need
/// most of them unless you're working on registration, federation, or media.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The user is authenticated but not allowed to perform the action.
    #[serde(rename = "M_FORBIDDEN")]
    Forbidden,
    /// The access token is not recognized (expired, revoked, or bogus).
    #[serde(rename = "M_UNKNOWN_TOKEN")]
    UnknownToken,
    /// No access token was supplied in the request.
    #[serde(rename = "M_MISSING_TOKEN")]
    MissingToken,
    /// The request body is JSON but doesn't match the expected schema.
    #[serde(rename = "M_BAD_JSON")]
    BadJson,
    /// The request body is not valid JSON (or `Content-Type` is wrong).
    #[serde(rename = "M_NOT_JSON")]
    NotJson,
    /// The requested resource was not found.
    #[serde(rename = "M_NOT_FOUND")]
    NotFound,
    /// Too many requests — the client should back off and retry later.
    #[serde(rename = "M_LIMIT_EXCEEDED")]
    LimitExceeded,
    /// An unknown error occurred (the server-side catch-all).
    #[serde(rename = "M_UNKNOWN")]
    Unknown,
    /// The server did not understand the request (unrecognized endpoint
    /// parameters, not a missing endpoint — that's a plain 404).
    #[serde(rename = "M_UNRECOGNIZED")]
    Unrecognized,
    /// Generic unauthorized (rarely used directly; prefer `UnknownToken`
    /// or `MissingToken` which are more specific).
    #[serde(rename = "M_UNAUTHORIZED")]
    Unauthorized,
    /// The user's account has been deactivated and can no longer be used.
    #[serde(rename = "M_USER_DEACTIVATED")]
    UserDeactivated,
    /// Registration failed: the desired user ID is already taken.
    #[serde(rename = "M_USER_IN_USE")]
    UserInUse,
    /// Registration failed: the desired username is invalid.
    #[serde(rename = "M_INVALID_USERNAME")]
    InvalidUsername,
    /// A room with the requested alias already exists.
    #[serde(rename = "M_ROOM_IN_USE")]
    RoomInUse,
    /// The room's state is invalid or inconsistent.
    #[serde(rename = "M_INVALID_ROOM_STATE")]
    InvalidRoomState,
    /// A third-party identifier (email/phone) is already bound to another user.
    #[serde(rename = "M_THREEPID_IN_USE")]
    ThreepidInUse,
    /// The provided third-party identifier is not associated with any user.
    #[serde(rename = "M_THREEPID_NOT_FOUND")]
    ThreepidNotFound,
    /// Third-party identifier authentication failed.
    #[serde(rename = "M_THREEPID_AUTH_FAILED")]
    ThreepidAuthFailed,
    /// The server policy does not allow the third-party identifier.
    #[serde(rename = "M_THREEPID_DENIED")]
    ThreepidDenied,
    /// Federation: the remote server is not on the trusted server list.
    #[serde(rename = "M_SERVER_NOT_TRUSTED")]
    ServerNotTrusted,
    /// The requested room version is not supported by this server.
    #[serde(rename = "M_UNSUPPORTED_ROOM_VERSION")]
    UnsupportedRoomVersion,
    /// The room version is known but incompatible with the requested action.
    #[serde(rename = "M_INCOMPATIBLE_ROOM_VERSION")]
    IncompatibleRoomVersion,
    /// The request would put the room into a bad state (e.g. no power
    /// level event).
    #[serde(rename = "M_BAD_STATE")]
    BadState,
    /// Guest access is not allowed for this room or action.
    #[serde(rename = "M_GUEST_ACCESS_FORBIDDEN")]
    GuestAccessForbidden,
    /// A CAPTCHA challenge is required to proceed.
    #[serde(rename = "M_CAPTCHA_NEEDED")]
    CaptchaNeeded,
    /// The provided CAPTCHA response was invalid.
    #[serde(rename = "M_CAPTCHA_INVALID")]
    CaptchaInvalid,
    /// A required parameter was missing from the request.
    #[serde(rename = "M_MISSING_PARAM")]
    MissingParam,
    /// A parameter was present but had an invalid value.
    #[serde(rename = "M_INVALID_PARAM")]
    InvalidParam,
    /// The request payload exceeds the server's size limit.
    #[serde(rename = "M_TOO_LARGE")]
    TooLarge,
    /// The resource is reserved by an application service.
    #[serde(rename = "M_EXCLUSIVE")]
    Exclusive,
    /// A server-wide resource limit has been reached (e.g. max users).
    #[serde(rename = "M_RESOURCE_LIMIT_EXCEEDED")]
    ResourceLimitExceeded,
    /// The user tried to leave the server notices room, which is not allowed.
    #[serde(rename = "M_CANNOT_LEAVE_SERVER_NOTICE_ROOM")]
    CannotLeaveServerNoticeRoom,
    /// The provided password does not meet the server's strength requirements.
    #[serde(rename = "M_WEAK_PASSWORD")]
    WeakPassword,
    /// Federation: the resident server could not authorize a restricted join.
    #[serde(rename = "M_UNABLE_TO_AUTHORISE_JOIN")]
    UnableToAuthoriseJoin,
    /// Federation: no resident server was able to grant the join.
    #[serde(rename = "M_UNABLE_TO_GRANT_JOIN")]
    UnableToGrantJoin,
    /// The user already sent the same annotation (reaction) to this event.
    #[serde(rename = "M_DUPLICATE_ANNOTATION")]
    DuplicateAnnotation,
    /// Media: the MXC URI was created but the content hasn't been uploaded yet.
    #[serde(rename = "M_NOT_YET_UPLOADED")]
    NotYetUploaded,
    /// Media: attempting to upload to an MXC URI that already has content.
    #[serde(rename = "M_CANNOT_OVERWRITE_MEDIA")]
    CannotOverwriteMedia,
    /// Sliding sync: the `pos` token is unknown or expired; the client
    /// must start a fresh sync.
    #[serde(rename = "M_UNKNOWN_POS")]
    UnknownPos,
    /// URL preview: the server does not have a URL preview service configured.
    #[serde(rename = "M_URL_NOT_SET")]
    UrlNotSet,
    /// The room alias in the request is malformed or doesn't resolve.
    #[serde(rename = "M_BAD_ALIAS")]
    BadAlias,
    /// MSC3706: The room's member list is not yet fully available because a
    /// partial-state join is still being resolved in the background.
    #[serde(rename = "org.matrix.msc3706.partial_state")]
    PartialState,
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
