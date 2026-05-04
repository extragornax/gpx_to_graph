use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect};
use axum::{Extension, Json};
use serde::Deserialize;

use super::{strava, AuthState, CurrentUser};

const LOGIN_HTML: &str = include_str!("../../static/auth/index.html");
const APP_CSS: &str = include_str!("../../static/auth/app.css");

pub async fn login_page() -> Html<String> {
    Html(LOGIN_HTML.replace("<!-- CSS_PLACEHOLDER -->", &format!("<style>{APP_CSS}</style>")))
}

pub async fn challenge() -> Json<crate::pow::Challenge> {
    Json(crate::pow::generate())
}

#[derive(Deserialize)]
pub(crate) struct AuthBody {
    username: String,
    password: String,
    pow: crate::pow::PowSolution,
}

pub async fn register(
    Extension(auth): Extension<AuthState>,
    Json(body): Json<AuthBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !crate::pow::verify(&body.pow) {
        return Err((StatusCode::BAD_REQUEST, "Invalid challenge".into()));
    }
    if body.username.len() < 2 || body.password.len() < 6 {
        return Err((StatusCode::BAD_REQUEST, "Username min 2 chars, password min 6 chars".into()));
    }
    if auth.db.get_user_by_username(&body.username).map_err(err500)?.is_some() {
        return Err((StatusCode::CONFLICT, "Username taken".into()));
    }

    let hash = super::hash_password(&body.password).map_err(err500)?;
    let user_id = auth.db.create_user(&body.username, &hash).map_err(err500)?;
    let token = super::generate_session_token();
    auth.db.create_session(&token, user_id).map_err(err500)?;

    Ok((
        StatusCode::CREATED,
        session_headers(&token),
        Json(serde_json::json!({ "username": body.username })),
    ))
}

pub async fn login(
    Extension(auth): Extension<AuthState>,
    Json(body): Json<AuthBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !crate::pow::verify(&body.pow) {
        return Err((StatusCode::BAD_REQUEST, "Invalid challenge".into()));
    }
    let (user_id, hash) = auth.db.get_user_by_username(&body.username)
        .map_err(err500)?
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid credentials".into()))?;

    if !super::verify_password(&body.password, &hash).map_err(err500)? {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into()));
    }

    let token = super::generate_session_token();
    auth.db.create_session(&token, user_id).map_err(err500)?;

    Ok((
        session_headers(&token),
        Json(serde_json::json!({ "username": body.username })),
    ))
}

pub async fn logout(
    Extension(auth): Extension<AuthState>,
    _user: CurrentUser,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(token) = extract_session_cookie(&headers) {
        auth.db.delete_session(token).map_err(err500)?;
    }
    let mut h = HeaderMap::new();
    h.insert(
        "set-cookie",
        "session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap(),
    );
    Ok((h, StatusCode::NO_CONTENT))
}

pub async fn me(user: CurrentUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "id": user.id, "username": user.username }))
}

// ── Strava OAuth ──

#[derive(Deserialize)]
pub struct StravaRedirectParams {
    redirect: Option<String>,
}

pub async fn strava_redirect(
    Extension(auth): Extension<AuthState>,
    _user: CurrentUser,
    Query(params): Query<StravaRedirectParams>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = auth.strava_config.as_ref()
        .ok_or((StatusCode::NOT_FOUND, "Strava integration not configured".into()))?;
    Ok(Redirect::temporary(&config.authorize_url(params.redirect.as_deref())))
}

#[derive(Deserialize)]
pub struct StravaCallbackParams {
    code: String,
    state: Option<String>,
}

pub async fn strava_callback(
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
    Query(params): Query<StravaCallbackParams>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = auth.strava_config.as_ref()
        .ok_or((StatusCode::NOT_FOUND, "Strava integration not configured".into()))?;

    let token = strava::exchange_code(config, &params.code).await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Strava token exchange failed: {e}")))?;

    let name = match (&token.athlete.firstname, &token.athlete.lastname) {
        (Some(f), Some(l)) => Some(format!("{f} {l}")),
        (Some(f), None) => Some(f.clone()),
        _ => None,
    };

    auth.db.save_strava_tokens(
        user.id, &token.access_token, &token.refresh_token,
        token.expires_at, token.athlete.id, name.as_deref(),
    ).map_err(err500)?;

    let redirect_to = params.state.as_deref().unwrap_or("/");
    Ok(Redirect::temporary(redirect_to))
}

pub async fn strava_status(
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let configured = auth.strava_config.is_some();
    let tokens = auth.db.get_strava_tokens(user.id).map_err(err500)?;

    Ok(Json(serde_json::json!({
        "configured": configured,
        "connected": tokens.is_some(),
        "athlete_name": tokens.as_ref().and_then(|t| t.athlete_name.clone()),
    })))
}

pub async fn strava_disconnect(
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
) -> Result<StatusCode, (StatusCode, String)> {
    auth.db.delete_strava_tokens(user.id).map_err(err500)?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "set-cookie",
        format!("session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000")
            .parse().unwrap(),
    );
    headers
}

fn extract_session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers.get("cookie")?
        .to_str().ok()?
        .split(';')
        .filter_map(|s| s.trim().strip_prefix("session="))
        .next()
}

fn err500(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
