use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chat_relay::{build_api_key_settings_json, get_vm_settings, set_vm_settings};
use serde::{Deserialize, Serialize};
use store::upsert_user;
use tower_sessions::Session;

use crate::{
    auth::User,
    handlers::{attach_csrf_token, validate_csrf},
    state::{AppError, AppState, find_user_vm_guest_ip},
};

#[derive(Serialize)]
pub(crate) struct SettingsResponse {
    uses_bedrock: bool,
    has_api_key: bool,
    base_url: Option<String>,
}

pub(crate) async fn get_settings_handler(
    user: User,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    if state.config.use_iam_creds {
        return Ok(Json(SettingsResponse {
            uses_bedrock: true,
            has_api_key: false,
            base_url: state.config.anthropic_base_url.clone(),
        })
        .into_response());
    }
    let db_user = upsert_user(&state.db, &user.email).await?;
    let guest_ip_opt = find_user_vm_guest_ip(&state.vms, db_user.id)?;
    let Some(guest_ip) = guest_ip_opt else {
        return Ok(Json(SettingsResponse {
            uses_bedrock: false,
            has_api_key: false,
            base_url: state.config.anthropic_base_url.clone(),
        })
        .into_response());
    };
    let vm_settings = get_vm_settings(
        guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
    )
    .await?;
    Ok(Json(SettingsResponse {
        uses_bedrock: false,
        has_api_key: vm_settings.has_api_key,
        base_url: state.config.anthropic_base_url.clone(),
    })
    .into_response())
}

#[derive(Deserialize)]
pub(crate) struct SetSettingsBody {
    api_key: String,
    csrf_token: String,
}

pub(crate) async fn put_settings_handler(
    user: User,
    session: Session,
    State(state): State<AppState>,
    Json(body): Json<SetSettingsBody>,
) -> Response {
    let Some(csrf_token) = validate_csrf(&session, &body.csrf_token).await else {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    };
    if state.config.use_iam_creds {
        return (
            StatusCode::BAD_REQUEST,
            "API key not applicable in Bedrock mode",
        )
            .into_response();
    }
    let db_user = match upsert_user(&state.db, &user.email).await {
        Ok(u) => u,
        Err(e) => return AppError::from(e).into_response(),
    };
    let guest_ip_opt = match find_user_vm_guest_ip(&state.vms, db_user.id) {
        Ok(ip) => ip,
        Err(e) => return AppError::from(e).into_response(),
    };
    let Some(guest_ip) = guest_ip_opt else {
        return (StatusCode::NOT_FOUND, "No active VM").into_response();
    };
    let content = build_api_key_settings_json(
        &body.api_key,
        state.config.anthropic_base_url.as_deref(),
        &state.config.anthropic_default_haiku_model,
        &state.config.anthropic_default_sonnet_model,
        &state.config.anthropic_default_opus_model,
    );
    if let Err(e) = set_vm_settings(
        guest_ip,
        &state.config.ssh_key_path,
        &state.config.ssh_user,
        &state.config.vm_host_key_path,
        &content,
    )
    .await
    {
        return AppError::from(e).into_response();
    }
    attach_csrf_token((StatusCode::OK, "").into_response(), &csrf_token)
}

