use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
struct WellKnownServerResponse {
	#[serde(rename = "m.server")]
	server: String,
}

#[derive(Serialize)]
struct WellKnownServerSupport {
	support_page: Option<String>,
	contacts: Vec<SupportContact>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct SupportContact {
	email_address: Option<String>,
	matrix_id: Option<String>,
	role: Option<String>,
}

async fn well_known_support(State(state): State<AppState>) -> impl IntoResponse {
	Json(WellKnownServerSupport {
		support_page: None,
		contacts: state.config.contact.iter().cloned().collect(),
	})
}
async fn well_known_server(State(state): State<AppState>) -> impl IntoResponse {
	Json(WellKnownServerResponse {
		server: state.config.delegate_url.clone(),
	})
}

#[derive(Deserialize, Debug)]
struct Config {
	contact: Option<SupportContact>,
	delegate_url: String,
}

#[derive(Clone)]
struct AppState {
	config: Arc<Config>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
	tracing_subscriber::fmt::init();
	let config: Config =
		serde_json::de::from_str(&tokio::fs::read_to_string("config.json").await?)?;
	let config = Arc::new(config);

	// TODO: path to server
	//  - /.well-known/matrix/server (reference to this server itself, with port)
	//  - /.well-known/matrix/support (support contact)
	//  - /_matrix/federation/v1/version (server version)
	//  - /_matrix/federation/v2/server (keys)
	//  - /_matrix/federation/v1/query/directory (alias resolution)
	//  - /_matrix/federation/v1/publicRooms (public room list)
	let router = Router::new()
		.route("/.well-known/matrix/server", get(well_known_server))
		.route("/.well-known/matrix/support", get(well_known_support))
		.with_state(AppState {
			config: config.clone(),
		});
	let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;

	axum::serve(listener, router).await?;
	Ok(())
}
