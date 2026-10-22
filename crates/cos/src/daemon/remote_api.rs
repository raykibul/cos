//! Remote access in the API: the device and listener routes (merged into
//! [`api::router`] on the Unix socket), and [`remote_router`], which serves
//! the whole API on the tailnet listener behind a [`Guard`].
//!
//! - `GET /v1/devices`: [`DevicesPage`]
//! - `POST /v1/devices/{id}/approve`, `POST /v1/devices/{id}/deny`: the [`Device`]
//! - `DELETE /v1/devices/{id}`: 204
//! - `GET /v1/tailnet`: [`TailnetStatus`]
//! - `PUT /v1/tailnet` with [`ConfigureTailnet`]: the [`TailnetStatus`] once
//!   the listener has started or stopped
//! - `GET /v1/tailnet/peers?port=N`: [`PeersPage`], the other devices on the
//!   tailnet and whether cos answers on port `N` (default: the listener's)
//!
//! Over the tailnet these answer 403 `host_only`, so a remote device can't
//! approve itself or switch the listener. Every other route is served there
//! too, including ones added later, once the guard lets the request in.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use cos_core::{Device, DeviceError};
use serde::{Deserialize, Serialize};

use super::api::{self, ApiError, AppState};
use super::peers::PeersPage;
use super::tailnet::{Guard, Refusal, TailnetSettings, TailnetStatus};

/// How long `PUT /v1/tailnet` waits for the listener to start or stop.
const SETTLE: Duration = Duration::from_secs(15);
/// Path prefixes only the Unix socket may use.
const HOST_ONLY: &[&str] = &["/v1/devices", "/v1/tailnet"];

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DevicesPage {
    pub devices: Vec<Device>,
}

/// Body of `PUT /v1/tailnet`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ConfigureTailnet {
    pub enabled: bool,
    /// Defaults to the current port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct PeersQuery {
    port: Option<u16>,
}

/// The device and listener routes, for the Unix socket.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/devices", get(list_devices))
        .route("/v1/devices/{id}", delete(remove_device))
        .route("/v1/devices/{id}/approve", post(approve_device))
        .route("/v1/devices/{id}/deny", post(deny_device))
        .route("/v1/tailnet", get(tailnet_status).put(configure_tailnet))
        .route("/v1/tailnet/peers", get(tailnet_peers))
}

/// The API on the tailnet listener: every route of [`api::router`] behind
/// `guard`, which admits only approved devices of this machine's Tailscale
/// user, with the host-only routes refused. The guard also covers unknown
/// paths, so a stranger can't even probe for 404s.
pub(crate) fn remote_router(state: AppState, guard: Arc<Guard>) -> Router {
    Router::new()
        .fallback_service(api::router(state))
        .layer(middleware::from_fn(refuse_host_only))
        .layer(middleware::from_fn_with_state(guard, admit))
}

/// Checks every remote request before anything else sees it.
async fn admit(
    State(guard): State<Arc<Guard>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    match guard.admit(addr).await {
        Ok(_) => next.run(request).await,
        Err(refusal) => refusal.into_response(),
    }
}

async fn refuse_host_only(request: Request, next: Next) -> Response {
    if is_host_only(request.uri().path()) {
        return Refusal {
            status: StatusCode::FORBIDDEN,
            code: "host_only",
            message:
                "devices and remote access can only be managed on the host (`cos devices`, `cos remote`)"
                    .into(),
        }
        .into_response();
    }
    next.run(request).await
}

fn is_host_only(path: &str) -> bool {
    HOST_ONLY.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }) || super::chief_api::is_tool_route(path)
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(error = %self.message, "remote request failed");
        }
        let body = Json(serde_json::json!({ "error": self.message, "code": self.code }));
        (self.status, body).into_response()
    }
}

async fn list_devices(State(state): State<AppState>) -> Result<Json<DevicesPage>, ApiError> {
    Ok(Json(DevicesPage {
        devices: state.devices.list()?,
    }))
}

async fn approve_device(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Device>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such device"))?;
    Ok(Json(state.devices.approve(id)?))
}

async fn deny_device(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Device>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such device"))?;
    Ok(Json(state.devices.deny(id)?))
}

async fn remove_device(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found("there is no such device"))?;
    state.devices.remove(id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn tailnet_status(State(state): State<AppState>) -> Json<TailnetStatus> {
    Json(state.tailnet.status())
}

/// The other devices on the tailnet. When Tailscale can't be used this is
/// still 200, with no peers and the reason.
async fn tailnet_peers(
    State(state): State<AppState>,
    query: Result<Query<PeersQuery>, QueryRejection>,
) -> Result<Json<PeersPage>, ApiError> {
    let Query(query) = query.map_err(|rejection| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: rejection.body_text(),
    })?;
    let port = query.port.unwrap_or(state.tailnet.settings().port);
    if port == 0 {
        return Err(port_out_of_range());
    }
    Ok(Json(state.peers.list(port).await))
}

fn port_out_of_range() -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "the port must be between 1 and 65535".into(),
    }
}

/// Switches remote access on or off, then waits briefly for the listener so
/// the answer says where it listens or why it can't.
async fn configure_tailnet(
    State(state): State<AppState>,
    body: Result<Json<ConfigureTailnet>, JsonRejection>,
) -> Result<Json<TailnetStatus>, ApiError> {
    let Json(body) = body?;
    let port = body.port.unwrap_or(state.tailnet.settings().port);
    if port == 0 {
        return Err(port_out_of_range());
    }
    state.tailnet.configure(TailnetSettings {
        enabled: body.enabled,
        port,
    })?;
    Ok(Json(state.tailnet.settled(SETTLE).await))
}

impl From<DeviceError> for ApiError {
    fn from(error: DeviceError) -> Self {
        let status = match &error {
            DeviceError::NotFound(_) => StatusCode::NOT_FOUND,
            DeviceError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_only_paths() {
        for path in [
            "/v1/devices",
            "/v1/devices/3/approve",
            "/v1/tailnet",
            "/v1/tailnet/",
            "/v1/tailnet/peers",
            "/v1/agents/3/tools/board_get",
        ] {
            assert!(is_host_only(path), "{path}");
        }
        for path in [
            "/v1/status",
            "/v1/devicesx",
            "/v1/tailnetwork",
            "/v2/devices",
            "/",
        ] {
            assert!(!is_host_only(path), "{path}");
        }
    }
}
