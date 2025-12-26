//! LSP request/notification test utilities

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tower_lsp::ClientSocket;
use tower_lsp::jsonrpc::Request;
use tower_lsp::lsp_types::*;

/// Create an LSP initialize request
pub fn create_initialize_request(id: i64) -> Request {
    Request::build("initialize")
        .id(id)
        .params(serde_json::to_value(InitializeParams::default()).unwrap())
        .finish()
}

/// Create an LSP initialized notification
pub fn create_initialized_notification() -> Request {
    Request::build("initialized")
        .params(serde_json::to_value(InitializedParams {}).unwrap())
        .finish()
}

/// Create an LSP didOpen notification
pub fn create_did_open_notification(uri: &str, content: &str) -> Request {
    Request::build("textDocument/didOpen")
        .params(
            serde_json::to_value(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.parse().unwrap(),
                    language_id: "yaml".to_string(),
                    version: 1,
                    text: content.to_string(),
                },
            })
            .unwrap(),
        )
        .finish()
}

/// Create an LSP didChange notification
#[allow(dead_code)]
pub fn create_did_change_notification(uri: &str, content: &str, version: i32) -> Request {
    Request::build("textDocument/didChange")
        .params(
            serde_json::to_value(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.parse().unwrap(),
                    version,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content.to_string(),
                }],
            })
            .unwrap(),
        )
        .finish()
}

/// Collect notifications in background and return a receiver
pub fn spawn_notification_collector(mut socket: ClientSocket) -> mpsc::Receiver<Request> {
    let (tx, rx) = mpsc::channel(100);

    tokio::spawn(async move {
        while let Some(notification) = socket.next().await {
            if tx.send(notification).await.is_err() {
                break;
            }
        }
    });

    rx
}

/// Wait for a notification with the specified method name from the receiver
pub async fn wait_for_notification(
    rx: &mut mpsc::Receiver<Request>,
    method: &str,
) -> Option<Request> {
    let timeout_duration = Duration::from_secs(5);

    loop {
        match timeout(timeout_duration, rx.recv()).await {
            Ok(Some(notification)) => {
                if notification.method() == method {
                    return Some(notification);
                }
                // Skip other notifications (like log_message)
            }
            _ => return None,
        }
    }
}

/// Create an LSP codeAction request
#[allow(dead_code)]
pub fn create_code_action_request(id: i64, uri: &str, line: u32, character: u32) -> Request {
    Request::build("textDocument/codeAction")
        .id(id)
        .params(
            serde_json::to_value(CodeActionParams {
                text_document: TextDocumentIdentifier {
                    uri: uri.parse().unwrap(),
                },
                range: Range {
                    start: Position { line, character },
                    end: Position { line, character },
                },
                context: CodeActionContext {
                    diagnostics: vec![],
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .unwrap(),
        )
        .finish()
}
