use axum::{
    extract::{Form, FromRequestParts, State},
    http::request::Parts,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{
    frontend::LOGIN_HTML,
    state::AppState,
};

pub(crate) struct User;

impl<S: Send + Sync> FromRequestParts<S> for User {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|e| e.into_response())?;
        match session.get::<bool>("logged_in").await {
            Ok(Some(true)) => Ok(User),
            _ => Err(Redirect::to("/login").into_response()),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    password: String,
}

pub(crate) async fn get_login_handler() -> Html<&'static str> {
    Html(LOGIN_HTML)
}

pub(crate) async fn post_login_handler(
    session: Session,
    State(state): State<AppState>,
    Form(login_form): Form<LoginForm>,
) -> Response {
    if login_form.password == state.demo_password {
        let _ = session.insert("logged_in", true).await;
        Redirect::to("/").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

pub(crate) async fn get_logout_handler(session: Session) -> impl IntoResponse {
    let _ = session.delete().await;
    Redirect::to("/login")
}
