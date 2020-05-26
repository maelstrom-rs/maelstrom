use std::convert::{TryFrom, From};

use actix_web::{
    http::StatusCode,
    web::{Data, Path},
    Error, HttpRequest, HttpResponse};

use ruma_identifiers::UserId;

use crate::{
    models::profile as profile_model,
    db::Store,
    server::error::{ResultExt, ErrorCode},
};

pub async fn get_displayname<T: Store>(
    req: Path<String>,
    storage: Data<T>,
) -> Result<HttpResponse, Error> {
    let user_id = UserId::try_from(req.into_inner())
        .with_codes(StatusCode::BAD_REQUEST, ErrorCode::INVALID_PARAM)?;
    let display_name = storage.fetch_display_name(&user_id).await
        .with_codes(StatusCode::NOT_FOUND, ErrorCode::UNKNOWN)?;

    Ok(HttpResponse::Ok()
        .json(profile_model::DisplayNameResponse {
            displayname: display_name,
    }))
}
