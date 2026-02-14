use axum::{
	Json, Router,
	extract::{Query, State},
	http::StatusCode,
	response::IntoResponse,
	routing::get,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, usize};
use tracing::error;

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

#[derive(Deserialize, Debug)]
struct Config {
	contact: Option<SupportContact>,
	delegate_url: String,
	public_rooms: Vec<PublicRoom>,
	aliases: HashMap<String, Alias>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum Alias {
	Direct {
		room_id: String,
		servers: Vec<String>,
	},
	Redirect {
		room_name: String,
		home_server: String,
	},
}

#[derive(Clone)]
struct AppState {
	config: Arc<Config>,
}

type Result<T, E = Report> = eyre::Result<T, E>;
struct Report(eyre::Report);
impl std::fmt::Debug for Report {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}
impl<E> From<E> for Report
where
	E: Into<eyre::Report>,
{
	fn from(value: E) -> Self {
		Self(value.into())
	}
}
impl IntoResponse for Report {
	fn into_response(self) -> axum::response::Response {
		let Self(err) = self;

		let err = match err.downcast::<MatrixError>() {
			Ok(intentional) => return (StatusCode::BAD_REQUEST, Json(intentional)).into_response(),
			Err(err) => err,
		};

		let error_msg = format!("{err}");

		error!(?err);

		(
			StatusCode::INTERNAL_SERVER_ERROR,
			Json(serde_json::json!({
				"errcode": "M_UNKNOWN",
				"error": error_msg,
			})),
		)
			.into_response()
	}
}

macro_rules! bail {
    ($($rest:tt)*) => {
		return Err(eyre::eyre!($($rest)*).into())
    };
}

macro_rules! ensure {
	($cond:expr, $($rest:tt)*) => {
		if !($cond) {
			bail!($($rest)*)
		}
	};
}

#[tokio::main]
async fn main() -> Result<()> {
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
		.route("/_matrix/federation/v1/version", get(version))
		.route("/_matrix/federation/v1/publicRooms", get(public_rooms))
		.route(
			"/_matrix/federation/v1/query/directory",
			get(query_directory),
		)
		.with_state(AppState {
			config: config.clone(),
		});
	let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;

	axum::serve(listener, router).await?;
	Ok(())
}
#[derive(Deserialize)]
struct RoomQuery {
	room_alias: String,
}
#[derive(Serialize, Clone, Deserialize)]
struct RoomQueryResponse {
	room_id: String,
	servers: Vec<String>,
}

async fn query_directory(
	Query(query): Query<RoomQuery>,
	State(state): State<AppState>,
) -> Result<impl IntoResponse> {
	match state.config.aliases.get(&query.room_alias) {
		Some(resolved) => match resolved {
			Alias::Direct { room_id, servers } => {
				return Ok(Json(RoomQueryResponse {
					room_id: room_id.clone(),
					servers: servers.clone(),
				}));
			}
			Alias::Redirect {
				room_name,
				home_server,
			} => {
				let result = match query_cache(home_server.clone(), room_name.clone()).await {
					Ok(o) => o,
					Err(e) => {
						bail!("Failed to resolve redirect alias");
					}
				};
				return Ok(Json(result));
			}
		},
		None => bail!(MatrixError {
			error: "M_NOT_FOUND".into(),
			errcode: format!("could not find alias for room {}", query.room_alias),
		}),
	}
}

use std::time::Duration;

#[cached::proc_macro::cached(time = 3600)]
async fn query_cache(
	home_server: String,
	room_name: String,
) -> Result<RoomQueryResponse, Arc<Report>> {
	async fn internal(home_server: &str, room_name: &str) -> Result<RoomQueryResponse> {
		let mut url = reqwest::Url::parse(&format!(
			"https://{home_server}/_matrix/client/v3/directory/room"
		))?;
		url.path_segments_mut().unwrap().push(&room_name);
		let resp = reqwest::get(url).await?;
		let result: RoomQueryResponse = resp.json().await?;
		let main_server = match result.servers.first() {
			Some(main_server) => main_server,
			None => bail!("missing servers in server list for {}", room_name),
		};
		let room_id = if result.room_id.contains(':') {
			result.room_id
		} else {
			format!("{}:{}", result.room_id, main_server)
		};

		Ok(RoomQueryResponse {
			room_id,
			servers: result.servers,
		})
	}
	match internal(&home_server, &room_name).await {
		Ok(o) => Ok(o),
		Err(e) => {
			error!(
				?e,
				"Error during resolution of {home_server} from {room_name}"
			);
			Err(Arc::new(e))
		}
	}
}

#[derive(Serialize, thiserror::Error, Debug)]
#[error("intentional matrix error")]
struct MatrixError {
	error: String,
	errcode: String,
}

#[derive(Deserialize)]
struct PublicRoomQuery {
	#[serde(default)]
	include_all_networks: bool,
	#[serde(default)]
	limit: usize,
	#[serde(default)]
	since: Option<String>,
}

#[derive(Serialize)]
struct PublicRoomResponse {
	chunk: Vec<PublicRoom>,
	#[serde(skip_serializing_if = "Option::is_none")]
	prev_batch: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	next_batch: Option<String>,
	total_room_count_estimate: usize,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
struct PublicRoom {
	#[serde(skip_serializing_if = "Option::is_none", default)]
	avatar_url: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none", default)]
	canonical_alias: Option<String>,
	guest_can_join: bool,
	#[serde(skip_serializing_if = "Option::is_none", default)]
	join_rule: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none", default)]
	name: Option<String>,
	num_joined_members: u32,
	room_id: String,
	room_type: String,
	#[serde(skip_serializing_if = "Option::is_none", default)]
	topic: Option<String>,
	world_readable: bool,
}

#[axum::debug_handler]
async fn public_rooms(
	Query(query): Query<PublicRoomQuery>,
	State(state): State<AppState>,
) -> Result<impl IntoResponse> {
	let offset: usize = match query.since {
		Some(s) => s.parse()?,
		None => 0,
	};
	let rooms = &state.config.public_rooms;
	let limit = if query.limit == 0 {
		usize::MAX
	} else {
		offset.saturating_add(query.limit)
	};
	ensure!(offset <= rooms.len(), "Since too big");
	let view = if query.include_all_networks {
		&rooms[offset..(limit.min(rooms.len()))]
	} else {
		&[]
	};
	Ok(Json(PublicRoomResponse {
		total_room_count_estimate: rooms.len(),
		chunk: view.into(),
		prev_batch: if limit == usize::MAX || view.is_empty() || offset == 0 {
			None
		} else {
			Some(offset.saturating_sub(limit).to_string())
		},
		next_batch: if limit == usize::MAX || view.is_empty() || limit >= rooms.len() {
			None
		} else {
			Some(offset.saturating_add(limit).to_string())
		},
	}))
}

async fn version() -> impl IntoResponse {
	Json(serde_json::json!({
		"server": {
			"name": "sbgg-matrix",
			"version": env!("CARGO_PKG_VERSION"),
		},
	}))
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
