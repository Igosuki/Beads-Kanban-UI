//! File watcher SSE endpoint for real-time file change notifications.
//!
//! Provides Server-Sent Events for monitoring changes to beads issue files.
//! Delegates to the backend registry to obtain a per-project watch stream.

use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, path::PathBuf, time::Duration};
use tokio_stream::StreamExt as _;

use crate::backend::detect::SharedBackendRegistry;

/// Query parameters for the watch endpoint.
#[derive(Debug, Deserialize)]
pub struct WatchParams {
    /// The project path to watch for changes.
    pub path: String,
}

/// File change event sent to clients.
#[derive(Debug, Serialize)]
pub struct FileChangeEvent {
    /// The path of the changed file.
    pub path: String,
    /// The type of change (modified, created, removed).
    #[serde(rename = "type")]
    pub change_type: String,
}

/// SSE endpoint for watching beads file changes.
///
/// Obtains a watch stream from the backend for the specified project path
/// and forwards events as Server-Sent Events to the client.
///
/// # Query Parameters
///
/// - `path`: The project directory path to monitor
///
/// # Returns
///
/// A Server-Sent Events stream of file change notifications.
pub async fn watch_beads(
    State(registry): State<SharedBackendRegistry>,
    Query(params): Query<WatchParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let project_path = PathBuf::from(&params.path);

    // Get or create the backend for this project.
    // If the registry fails, return an empty stream (the SSE connection will
    // close immediately — the client will reconnect and retry).
    let backend = {
        let mut reg = registry.write().await;
        reg.get_or_create(&project_path).ok()
    };

    let stream = match backend {
        Some(b) => {
            // Obtain the raw FileChangeEvent stream from the backend, then map
            // each event to an SSE Event with JSON-encoded data.
            let raw = b.watch(project_path);
            let sse_stream = raw.map(|event| {
                let data = serde_json::to_string(&event).unwrap_or_default();
                Ok::<Event, Infallible>(Event::default().data(data))
            });
            // Box to unify types for the two branches.
            Box::pin(sse_stream) as std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>
        }
        None => {
            // Backend unavailable — return an empty stream.
            Box::pin(futures::stream::empty()) as std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_change_event_serialization() {
        let event = FileChangeEvent {
            path: "/test/path".to_string(),
            change_type: "modified".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"path\":\"/test/path\""));
        assert!(json.contains("\"type\":\"modified\""));
    }

    #[test]
    fn test_watch_params_deserialization() {
        let params: WatchParams =
            serde_json::from_str(r#"{"path": "/test/project"}"#).unwrap();
        assert_eq!(params.path, "/test/project");
    }
}
