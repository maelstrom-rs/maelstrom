use std::convert::TryInto;

use ruma_identifiers::{DeviceId, UserId};
use serde::Deserialize;

use crate::CONFIG;

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
    pub device_id: Option<DeviceId>,
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

/// Checks to see if the username is valid and does NOT
/// contain any non-allowed characters
pub fn is_username_valid(username: &str) -> bool {
    let res: Result<UserId, _> = format!("@{}:{}", username, CONFIG.hostname)[..].try_into();
    dbg!(&res);
    // Shouldn't be able to register new names with historical characters
    res.is_ok() && !res.unwrap().is_historical()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_username_valid_good() {
        crate::init_config_from_file(".env-test");
        let good_username = "good_user";
        assert!(is_username_valid(good_username));
    }

    #[test]
    fn test_check_username_valid_bad() {
        crate::init_config_from_file(".env-test");
        let bad_username = "b@dn!ame$";
        assert_ne!(true, is_username_valid(bad_username));
    }

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
