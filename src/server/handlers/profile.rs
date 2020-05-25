use std::convert::{TryFrom, From};

use actix_web::{
    web::{Data, Path},
    Error, HttpRequest, HttpResponse};

use ruma_identifiers::UserId;

use crate::{
    models::profile as profile_model,
    db::Store
};

pub async fn get_displayname<T: Store>(
    req: Path<String>,
    storage: Data<T>,
) -> Result<HttpResponse, Error> {
    let userId = UserId::try_from(req.into_inner()).unwrap();
    Ok(HttpResponse::Ok()
        .json(profile_model::DisplayNameResponse {
            displayname: String::from(userId),
    }))
}
