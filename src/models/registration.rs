use serde::Deserialize;

/// The kind of account to register.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// An Anonymous user with no password.
    Guest,
    /// A regular user with password.
    User,
}

impl Kind {
    /// Creates a new `Kind` from a str.
    pub fn from_str(kind: &str) -> Self {
        match kind.to_lowercase().as_ref() {
            "guest" => Kind::Guest,
            _ => Kind::User,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RequestParams {
    pub kind: Option<Kind>,
}

#[derive(Deserialize)]
pub struct AvailableParams {
    pub username: String,
}

// TODO: Support `auth` and `authentication_data` fields
#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    /// ID of the client device. If this does not correspond to a known
    /// client device, a new device will be created. The server will
    /// auto-generate a device_id if this is not specified.
    pub device_id: Option<String>,
    /// If true, an `access_token` and `device_id` should not be returned
    /// from this call, therefore preventing an automatic login. Defaults
    /// to `false`.
    pub inhibit_login: Option<bool>,
    /// A display name to assign to the newly-created device. Ignored if
    /// `device_id` corresponds to a known device.
    pub initial_device_display_name: Option<String>,
    /// The desired password for the account.
    pub password: Option<String>,
    /// The type of user being registered, either `guest` or `user`.  Defaults
    /// to `user`.
    pub kind: Option<Kind>,
    /// The basis for the localpart of the desired Matrix ID. If omitted,
    /// the homeserver MUST generate a Matrix ID local part.
    pub username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_from_str_guest() {
        assert_eq!(Kind::from_str("GUEST"), Kind::Guest);
        assert_eq!(Kind::from_str("Guest"), Kind::Guest);
        assert_eq!(Kind::from_str("guest"), Kind::Guest);
    }

    #[test]
    fn test_kind_from_str_user() {
        assert_eq!(Kind::from_str("USER"), Kind::User);
        assert_eq!(Kind::from_str("User"), Kind::User);
        assert_eq!(Kind::from_str("user"), Kind::User);
    }

    #[test]
    fn test_kind_from_str_defaults_to_user() {
        assert_eq!(Kind::from_str(""), Kind::User);
        assert_eq!(Kind::from_str(" "), Kind::User);
        assert_eq!(Kind::from_str("bleh"), Kind::User);
    }
}
