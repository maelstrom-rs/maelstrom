use std::borrow::Cow;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};

use super::{Error, Store};
use crate::models::auth::{PWHash, UserIdentifier};

/// A Mock Storage engine used for Testing.
#[derive(Clone, Default)]
pub struct MockStore {
    pub check_username_exists_resp: Option<Result<bool, Error>>,
}

impl MockStore {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Default::default()
    }
}

#[async_trait]
impl Store for MockStore {
    fn get_type(&self) -> String {
        "MockStore".to_string()
    }

    async fn check_username_exists(&self, _username: &str) -> Result<bool, Error> {
        self.check_username_exists_resp
            .clone()
            .expect("check_username_exists_resp not set.")
    }

    async fn fetch_user_id<'a>(
        &self,
        user_id: &'a UserIdentifier,
    ) -> Result<Option<Cow<'a, UserId>>, Error> {
        unimplemented!()
    }

    async fn fetch_password_hash(&self, _user_id: &UserId) -> Result<PWHash, Error> {
        unimplemented!()
    }

    async fn check_otp_exists(&self, _user_id: &UserId, _otp: &str) -> Result<bool, Error> {
        unimplemented!()
    }

    async fn set_device<'a>(
        &self,
        _user_id: &UserId,
        _device_id: &DeviceId,
        _display_name: Option<&str>,
    ) -> Result<(), Error> {
        unimplemented!()
    }
}
