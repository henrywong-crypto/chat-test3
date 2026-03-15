use axum::{
    extract::{FromRequestParts, Query, State},
    http::{StatusCode, request::Parts},
    response::{Html, IntoResponse, Redirect, Response},
};
use handlers::{AppState as CognitoState, CallbackQuery, callback, login};
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
        session
            .get::<String>("email")
            .await
            .ok()
            .flatten()
            .map(|email| User { email })
            .ok_or_else(|| Redirect::to("/login").into_response())
    }
}

fn build_cognito_state(state: &AppState) -> CognitoState {
    CognitoState {
        client_id: state.config.cognito_client_id.clone(),
        client_secret: state.config.cognito_client_secret.clone(),
        domain: state.config.cognito_domain.clone(),
        redirect_uri: state.config.cognito_redirect_uri.clone(),
        region: state.config.cognito_region.clone(),
        user_pool_id: state.config.cognito_user_pool_id.clone(),
    }
}

pub(crate) async fn get_login_handler() -> Html<String> {
    Html(render_login_page())
}

pub(crate) async fn get_cognito_login_handler(
    session: Session,
    State(state): State<AppState>,
) -> Response {
    let cognito_state = build_cognito_state(&state);
    login(session, State(cognito_state))
        .await
        .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
}

pub(crate) async fn get_callback_handler(
    query: Query<CallbackQuery>,
    session: Session,
    State(state): State<AppState>,
) -> Response {
    let cognito_state = build_cognito_state(&state);
    callback(query, session, State(cognito_state))
        .await
        .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
}

pub(crate) async fn get_logout_handler(session: Session) -> impl IntoResponse {
    let _ = session.delete().await;
    Redirect::to("/login")
}
