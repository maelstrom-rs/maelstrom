use std::borrow::Cow;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};

use super::{Error, Store};
use crate::models::auth::{PWHash, UserIdentifier};

/// A Mock Storage engine used for Testing.
#[derive(Clone, Default)]
pub struct MockStore {
    pub check_username_exists_resp: Option<Result<bool, Error>>,
    pub check_device_id_exists_resp: Option<Result<bool, Error>>,
    pub remove_device_id_resp: Option<Result<(), Error>>,
    pub remove_all_device_ids_resp: Option<Result<(), Error>>,
}

impl MockStore {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_username_exists_resp(self, resp: Result<bool, Error>) -> MockStore {
        MockStore {
            check_username_exists_resp: Some(resp),
            ..self
        }
    }

    pub fn with_check_device_id_exists_resp(self, resp: Result<bool, Error>) -> MockStore {
        MockStore {
            check_device_id_exists_resp: Some(resp),
            ..self
        }
    }

    pub fn with_remove_device_id_resp(self, resp: Result<(), Error>) -> MockStore {
        MockStore {
            remove_device_id_resp: Some(resp),
            ..self
        }
    }

    pub fn with_remove_all_device_ids_resp(self, resp: Result<(), Error>) -> MockStore {
        MockStore {
            remove_all_device_ids_resp: Some(resp),
            ..self
        }
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

    async fn check_device_id_exists(&self, device_id: &DeviceId) -> Result<bool, Error> {
        self.check_device_id_exists_resp
            .clone()
            .expect("check_device_id_exists_resp not set.")
    }

    async fn remove_device_id(&self, device_id: &DeviceId, user_id: &UserId) -> Result<(), Error> {
        self.remove_device_id_resp
            .clone()
            .expect("remove_device_id_resp not set.")
    }

    async fn remove_all_device_ids(&self, user_id: &UserId) -> Result<(), Error> {
        self.remove_all_device_ids_resp
            .clone()
            .expect("remove_all_device_ids_resp not set.")
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
