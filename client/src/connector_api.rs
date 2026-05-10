use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthResponse {
    pub nickname: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignalingReadyResponse {
    pub signaling_url: String,
    pub session_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceErrorPayload {
    message: String,
}

#[derive(Debug, Serialize)]
struct AuthRequest<'a> {
    nickname: &'a str,
}

#[derive(Debug, Serialize)]
struct CreateRequest<'a> {
    session_id: &'a str,
}

#[derive(Debug, Serialize)]
struct JoinRequest<'a> {
    session_id: &'a str,
    room_id: &'a str,
}

#[derive(Debug, Serialize)]
struct LogoutRequest<'a> {
    session_id: &'a str,
}

#[derive(Debug, Serialize)]
struct SessionRenewRequest<'a> {
    session_id: &'a str,
}

/// Refreshes Redis session TTL while the user is idle on HTTP (nick keys no longer expire).
/// Call periodically (e.g. before `SESSION_TTL_SEC` elapses) when not connected over WebSocket.
pub async fn renew_session(session_id: &str) -> Result<(), String> {
    let body = serde_json::to_string(&SessionRenewRequest { session_id })
        .map_err(|err| err.to_string())?;
    let response = post_json("/session/renew", body).await?;
    parse_empty_response(response).await
}

pub async fn auth(nickname: &str) -> Result<AuthResponse, String> {
    let body = serde_json::to_string(&AuthRequest { nickname }).map_err(|err| err.to_string())?;
    let response = post_json("/auth", body).await?;
    parse_json_response(response).await
}

pub async fn create(session_id: &str) -> Result<SignalingReadyResponse, String> {
    let body =
        serde_json::to_string(&CreateRequest { session_id }).map_err(|err| err.to_string())?;
    let response = post_json("/create", body).await?;
    parse_json_response(response).await
}

pub async fn join(session_id: &str, room_id: &str) -> Result<SignalingReadyResponse, String> {
    let body = serde_json::to_string(&JoinRequest {
        session_id,
        room_id,
    })
    .map_err(|err| err.to_string())?;
    let response = post_json("/join", body).await?;
    parse_json_response(response).await
}

pub fn logout_best_effort(session_id: String) {
    spawn_local(async move {
        let body = match serde_json::to_string(&LogoutRequest {
            session_id: &session_id,
        }) {
            Ok(body) => body,
            Err(_) => return,
        };
        let Ok(request) = Request::post(&endpoint("/logout"))
            .header("content-type", "application/json")
            .body(body)
        else {
            return;
        };
        let _ = request.send().await;
    });
}

async fn post_json(path: &str, body: String) -> Result<gloo_net::http::Response, String> {
    Request::post(&endpoint(path))
        .header("content-type", "application/json")
        .body(body)
        .map_err(|err| err.to_string())?
        .send()
        .await
        .map_err(|err| err.to_string())
}

async fn parse_empty_response(response: gloo_net::http::Response) -> Result<(), String> {
    let status = response.status();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        match response.json::<ServiceErrorPayload>().await {
            Ok(err) => Err(err.message),
            Err(_) => Err(status_error_message(status).to_owned()),
        }
    }
}

async fn parse_json_response<T>(response: gloo_net::http::Response) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();
    if (200..300).contains(&status) {
        response
            .json::<T>()
            .await
            .map_err(|_| "Internal error: invalid service response format.".to_owned())
    } else {
        match response.json::<ServiceErrorPayload>().await {
            Ok(err) => Err(err.message),
            Err(_) => Err(status_error_message(status).to_owned()),
        }
    }
}

fn endpoint(path: &str) -> String {
    match option_env!("CONNECTOR_BASE_URL") {
        Some(base) if !base.is_empty() => format!("{}{}", base.trim_end_matches('/'), path),
        _ => path.to_owned(),
    }
}

fn status_error_message(status: u16) -> &'static str {
    match status {
        500 => "Internal error. Please try again.",
        502 | 503 | 504 => "Service unavailable. Please try again.",
        _ => "Service error. Please try again.",
    }
}
