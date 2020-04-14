use actix_web::{http::StatusCode, Error, HttpResponse};

#[derive(Clone, Debug, serde::Serialize)]
pub struct MatrixError {
    #[serde(skip)]
    pub status: StatusCode,
    pub errcode: ErrorCode,
    pub error: String,
}
impl From<MatrixError> for Error {
    fn from(e: MatrixError) -> Self {
        HttpResponse::build(e.status).json(e).into()
    }
}

pub trait ResultExt<T>: Sized {
    fn with_codes(self, status: StatusCode, code: ErrorCode) -> Result<T, MatrixError>;
    fn unknown(self) -> Result<T, MatrixError> {
        self.with_codes(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::UNKNOWN)
    }
}

impl<T, E> ResultExt<T> for Result<T, E>
where
    E: std::fmt::Display,
{
    fn with_codes(self, status: StatusCode, code: ErrorCode) -> Result<T, MatrixError> {
        self.map_err(|e| MatrixError {
            status,
            errcode: code,
            error: format!("{}", e),
        })
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[allow(non_camel_case_types)]
pub enum ErrorCode {
    #[serde(rename = "M_FORBIDDEN")]
    FORBIDDEN, // 	Forbidden access, e.g. joining a room without permission, failed login.
    #[serde(rename = "M_UNKNOWN_TOKEN")]
    UNKNOWN_TOKEN, //     The access token specified was not recognised.
    #[serde(rename = "M_MISSING_TOKEN")]
    MISSING_TOKEN, //     No access token was specified for the request.
    #[serde(rename = "M_BAD_JSON")]
    BAD_JSON, // 	Request contained valid JSON, but it was malformed in some way, e.g. missing required keys, invalid values for keys.
    #[serde(rename = "M_NOT_JSON")]
    NOT_JSON, // 	Request did not contain valid JSON.
    #[serde(rename = "M_NOT_FOUND")]
    NOT_FOUND, // 	No resource was found for this request.
    #[serde(rename = "M_LIMIT_EXCEEDED")]
    LIMIT_EXCEEDED, //  	Too many requests have been sent in a short period of time. Wait a while then try again.
    #[serde(rename = "M_UNKNOWN")]
    UNKNOWN, // 	An unknown error has occurred.
    #[serde(rename = "M_UNRECOGNIZED")]
    UNRECOGNIZED, // 	The server did not understand the request.
    #[serde(rename = "M_UNAUTHORIZED")]
    UNAUTHORIZED, // 	The request was not correctly authorized. Usually due to login failures.
    #[serde(rename = "M_USER_DEACTIVATED")]
    USER_DEACTIVATED, //  	The user ID associated with the request has been deactivated. Typically for endpoints that prove authentication, such as /login.
    #[serde(rename = "M_USER_IN_USE")]
    USER_IN_USE, // 	Encountered when trying to register a user ID which has been taken.
    #[serde(rename = "M_INVALID_USERNAME")]
    INVALID_USERNAME, //  	Encountered when trying to register a user ID which is not valid.
    #[serde(rename = "M_ROOM_IN_USE")]
    ROOM_IN_USE, // 	Sent when the room alias given to the createRoom API is already in use.
    #[serde(rename = "M_INVALID_ROOM_STATE")]
    INVALID_ROOM_STATE, //  	Sent when the initial state given to the createRoom API is invalid.
    #[serde(rename = "M_THREEPID_IN_USE")]
    THREEPID_IN_USE, //  	Sent when a threepid given to an API cannot be used because the same threepid is already in use.
    #[serde(rename = "M_THREEPID_NOT_FOUND")]
    THREEPID_NOT_FOUND, //  	Sent when a threepid given to an API cannot be used because no record matching the threepid was found.
    #[serde(rename = "M_THREEPID_AUTH_FAILED")]
    THREEPID_AUTH_FAILED, //  	Authentication could not be performed on the third party identifier.
    #[serde(rename = "M_THREEPID_DENIED")]
    THREEPID_DENIED, //  	The server does not permit this third party identifier. This may happen if the server only permits, for example, email addresses from a particular domain.
    #[serde(rename = "M_SERVER_NOT_TRUSTED")]
    SERVER_NOT_TRUSTED, //  	The client's request used a third party server, eg. identity server, that this server does not trust.
    #[serde(rename = "M_UNSUPPORTED_ROOM_VERSION")]
    UNSUPPORTED_ROOM_VERSION, //  	The client's request to create a room used a room version that the server does not support.
    #[serde(rename = "M_INCOMPATIBLE_ROOM_VERSION")]
    INCOMPATIBLE_ROOM_VERSION, //  	The client attempted to join a room that has a version the server does not support. Inspect the room_version property of the error response for the room's version.
    #[serde(rename = "M_BAD_STATE")]
    BAD_STATE, // 	The state change requested cannot be performed, such as attempting to unban a user who is not banned.
    #[serde(rename = "M_GUEST_ACCESS_FORBIDDEN")]
    GUEST_ACCESS_FORBIDDEN, //  	The room or resource does not permit guests to access it.
    #[serde(rename = "M_CAPTCHA_NEEDED")]
    CAPTCHA_NEEDED, //  	A Captcha is required to complete the request.
    #[serde(rename = "M_CAPTCHA_INVALID")]
    CAPTCHA_INVALID, //  	The Captcha provided did not match what was expected.
    #[serde(rename = "M_MISSING_PARAM")]
    MISSING_PARAM, //  	A required parameter was missing from the request.
    #[serde(rename = "M_INVALID_PARAM")]
    INVALID_PARAM, //  	A parameter that was specified has the wrong value. For example, the server expected an integer and instead received a string.
    #[serde(rename = "M_TOO_LARGE")]
    TOO_LARGE, // 	The request or entity was too large.
    #[serde(rename = "M_EXCLUSIVE")]
    EXCLUSIVE, // 	The resource being requested is reserved by an application service, or the application service making the request has not created the resource.
    #[serde(rename = "M_RESOURCE_LIMIT_EXCEEDED")]
    RESOURCE_LIMIT_EXCEEDED, //  	The request cannot be completed because the homeserver has reached a resource limit imposed on it. For example, a homeserver held in a shared hosting environment may reach a resource limit if it starts using too much memory or disk space. The error MUST have an admin_contact field to provide the user receiving the error a place to reach out to. Typically, this error will appear on routes which attempt to modify state (eg: sending messages, account data, etc) and not routes which only read state (eg: /sync, get account data, etc).
    #[serde(rename = "M_CANNOT_LEAVE_SERVER_NOTICE_ROOM")]
    CANNOT_LEAVE_SERVER_NOTICE_ROOM, //  	The user is unable to reject an invite to join the server notices room. See the Server Notices module for more information.
}
