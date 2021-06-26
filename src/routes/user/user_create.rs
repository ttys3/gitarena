use actix_web::{HttpResponse, post, Responder, web};
use crate::user::User;
use crate::{captcha, PgPoolConnection, templates};
use gitarena_proc_macro::generate_bail;
use log::error;
use log::info;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgQueryAs;
use sqlx::{Connection, PgPool, Transaction};
use std::collections::HashMap;

generate_bail!(RegisterJsonResponse {
                   success: false,
                   id: None,
                   errors: Some("Internal server error occurred".to_owned())
               });

#[post("/api/user")]
pub(crate) async fn register(body: web::Json<RegisterJsonRequest>, db_pool: web::Data<PgPool>) -> impl Responder {
    let connection: PgPoolConnection = bail!(db_pool.acquire().await);
    let mut transaction: Transaction<PgPoolConnection> = bail!(connection.begin().await);

    let username = validate_username(&body.username); // todo: check if empty
    let lowered_username = username.to_lowercase();

    let (exists,): (bool,) = bail!(sqlx::query_as("select exists(select 1 from users where lower(username) = $1);")
        .bind(&lowered_username)
        .fetch_one(&mut transaction)
        .await);

    if exists {
        return HttpResponse::Conflict().json(RegisterJsonResponse {
            success: false,
            id: None,
            errors: Some("Username already in use".to_owned())
        }).await;
    }

    let captcha_success = true/*bail!(captcha::verify_captcha(&body.h_captcha_response.to_owned()).await)*/;

    if !captcha_success {
        return HttpResponse::UnprocessableEntity().json(RegisterJsonResponse {
            success: false,
            id: None,
            errors: Some("Captcha verification failed".to_owned())
        }).await;
    }

    let mut user: User = bail!(User::new(
        username.to_owned(), body.email.to_owned(), body.password.to_owned()
    ));
    bail!(user.save(db_pool.get_ref()).await);

    bail!(transaction.commit().await);

    bail!(user.send_template(&templates::VERIFY_EMAIL, Some([
            ("username".to_owned(), user.username.to_owned()),
            ("link".to_owned(), "bruuh4".to_owned())
    ].iter().cloned().collect())).await);

    info!("New user registered: {} (id {})", user.username, user.id);

    HttpResponse::Ok().json(RegisterJsonResponse {
        success: true,
        id: Some(user.id),
        errors: None
    }).await
}

fn validate_username(username: &String) -> String {
    let regex = Regex::new("[^\\u0000-\\u007F]+").unwrap();

    match regex.find(username) {
        Some(matched_text) => matched_text.as_str().to_owned(),
        None => "".to_owned()
    }.replace(" ", "-")
}

#[derive(Deserialize)]
pub(crate) struct RegisterJsonRequest {
    username: String,
    email: String,
    password: String,
    h_captcha_response: String
}

#[derive(Serialize)]
struct RegisterJsonResponse {
    success: bool,
    id: Option<i32>,
    errors: Option<String>
}
