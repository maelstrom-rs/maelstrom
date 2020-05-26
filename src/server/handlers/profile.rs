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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::mock::MockStore,
        models::profile as profile_model,
    };

    use actix_service::Service;
    use actix_web::{http, test, web, App};

    #[actix_rt::test]
    async fn test_get_display_name_succeeds() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new())
                .route("/{userId}/displayname", web::get().to(get_displayname::<MockStore>))
        ).await;

        let req = test::TestRequest::get()
            .uri("/@testId:testServer/displayname")
            .to_request();

        let mut resp = app.call(req).await.unwrap();
        assert!(resp.status().is_success());

        let expected_body = profile_model::DisplayNameResponse {
                displayname: String::from("testDisplayName"),
            };

        let body: profile_model::DisplayNameResponse = serde_json::from_slice(&test::read_body(resp).await)
            .unwrap_or_else(|_| panic!("Couldn't deserialize response"));

        assert_eq!(body, expected_body);
    }
}
