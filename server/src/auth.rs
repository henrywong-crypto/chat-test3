use axum::{
    extract::{FromRequestParts, Query, State},
    http::{request::Parts, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use handlers::{callback, login, AppState as CognitoState, CallbackQuery};
use tower_sessions::Session;

use crate::{state::AppState, templates::render_login_page};

pub(crate) struct User {
    pub(crate) email: String,
}

impl<S: Send + Sync> FromRequestParts<S> for User {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|session_error| session_error.into_response())?;
        match session.get::<String>("email").await {
            Ok(Some(email)) => Ok(User { email }),
            _ => Err(Redirect::to("/login").into_response()),
        }
    }
}

fn build_cognito_state(state: &AppState) -> CognitoState {
    CognitoState {
        client_id: state.cognito_client_id.clone(),
        client_secret: state.cognito_client_secret.clone(),
        domain: state.cognito_domain.clone(),
        redirect_uri: state.cognito_redirect_uri.clone(),
        region: state.cognito_region.clone(),
        user_pool_id: state.cognito_user_pool_id.clone(),
    }
}

pub(crate) async fn get_login_handler() -> Html<String> {
    Html(render_login_page().into_string())
}

pub(crate) async fn get_cognito_login_handler(
    session: Session,
    State(state): State<AppState>,
) -> Response {
    let cognito_state = build_cognito_state(&state);
    match login(session, State(cognito_state)).await {
        Ok(response) => response,
        Err(login_error) => {
            (StatusCode::INTERNAL_SERVER_ERROR, login_error.to_string()).into_response()
        }
    }
}

pub(crate) async fn get_demo_handler(session: Session) -> impl IntoResponse {
    let _ = session.insert("email", "demo").await;
    Redirect::to("/sessions")
}

pub(crate) async fn get_callback_handler(
    query: Query<CallbackQuery>,
    session: Session,
    State(state): State<AppState>,
) -> Response {
    let cognito_state = build_cognito_state(&state);
    match callback(query, session, State(cognito_state)).await {
        Ok(response) => response,
        Err(callback_error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            callback_error.to_string(),
        )
            .into_response(),
    }
}

pub(crate) async fn get_logout_handler(session: Session) -> impl IntoResponse {
    let _ = session.delete().await;
    Redirect::to("/login")
}
