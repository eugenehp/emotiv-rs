use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub async fn spawn_mock_cortex_server(drop_first_connection: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let mut connection_count = 0_u32;

        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            connection_count += 1;
            let should_drop = drop_first_connection && connection_count == 1;

            tokio::spawn(async move {
                let Ok(mut ws) = accept_async(stream).await else { return };

                while let Some(Ok(msg)) = ws.next().await {
                    let Message::Text(text) = msg else { continue };
                    let Ok(req): Result<serde_json::Value, _> =
                        serde_json::from_str(text.as_ref())
                    else {
                        continue;
                    };

                    let id = req.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

                    let response = match method {
                        "hasAccessRight" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"accessGranted": true}
                        }),
                        "authorize" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"cortexToken": "mock-token"}
                        }),
                        "refreshHeadsets" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"message": "ok"}
                        }),
                        "queryHeadsets" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": [{"id": "MOCK-HS", "status": "connected"}]
                        }),
                        "createSession" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"id": "mock-session"}
                        }),
                        "subscribe" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "success": [{"streamName": "eeg", "cols": ["AF3", "F7", "F3"]}],
                                "failure": []
                            }
                        }),
                        "getCortexInfo" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"version": "mock-1.0"}
                        }),
                        _ => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {}
                        }),
                    };

                    let _ = ws.send(Message::Text(response.to_string().into())).await;

                    if method == "subscribe" {
                        let eeg = json!({
                            "sid": "mock-session",
                            "time": 123.0,
                            "eeg": [1.0, 2.0, 3.0]
                        });
                        let _ = ws.send(Message::Text(eeg.to_string().into())).await;
                    }

                    if method == "getCortexInfo" {
                        let _ = ws.close(None).await;
                        break;
                    }

                    if should_drop && method == "createSession" {
                        let _ = ws.close(None).await;
                        break;
                    }
                }
            });
        }
    });

    format!("ws://{}", addr)
}
