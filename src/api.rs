use crate::state::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum::http::{header, StatusCode, Uri};
use futures::stream::{self, Stream};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn ports(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.snapshot().await)
}

pub async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let initial = state.snapshot().await;
    let rx = state.subscribe();
    let initial_stream = stream::once(async move { Ok(Event::default().json_data(&initial).unwrap()) });
    let live = BroadcastStream::new(rx).filter_map(|res| {
        res.ok().map(|snap| Ok(Event::default().json_data(&snap).unwrap()))
    });
    Sse::new(initial_stream.chain(live)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[derive(rust_embed::Embed)]
#[folder = "assets/"]
struct Assets;

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
