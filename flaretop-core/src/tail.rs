//! Live logs: `POST .../tails` returns a WebSocket URL; connect with the
//! `trace-v1` sub-protocol, send `{"debug":false}` once, ping every 10 s,
//! and re-create the tail before `expires_at`. Delete the tail on exit.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::api::{ApiResult, Client};

#[derive(Debug, Clone, Deserialize)]
pub struct Tail {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TailFilters {
    /// ok | error (expands to exception, exceededCpu, exceededMemory, unknown) | canceled
    pub status: Vec<String>,
    pub methods: Vec<String>,
    pub search: Option<String>,
    pub sampling_rate: Option<f64>,
}

impl TailFilters {
    pub fn to_json(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if !self.status.is_empty() {
            let mut outcomes: Vec<&str> = Vec::new();
            for s in &self.status {
                match s.as_str() {
                    "ok" => outcomes.push("ok"),
                    "canceled" => outcomes.push("canceled"),
                    "error" => {
                        outcomes.extend(["exception", "exceededCpu", "exceededMemory", "unknown"])
                    }
                    other => outcomes.push(other),
                }
            }
            out.push(json!({ "outcome": outcomes }));
        }
        if !self.methods.is_empty() {
            out.push(json!({ "method": self.methods }));
        }
        if let Some(q) = &self.search
            && !q.is_empty()
        {
            out.push(json!({ "query": q }));
        }
        if let Some(r) = self.sampling_rate {
            out.push(json!({ "sampling_rate": r }));
        }
        out
    }
}

pub async fn create(
    c: &Client,
    account: &str,
    script: &str,
    filters: &TailFilters,
) -> ApiResult<Tail> {
    c.post(
        &format!("/accounts/{account}/workers/scripts/{script}/tails"),
        json!({ "filters": filters.to_json() }),
    )
    .await
}

pub async fn delete(c: &Client, account: &str, script: &str, tail_id: &str) -> ApiResult<Value> {
    c.delete(&format!(
        "/accounts/{account}/workers/scripts/{script}/tails/{tail_id}"
    ))
    .await
}

/// A decoded tail frame, kept close to the wire shape.
#[derive(Debug, Clone)]
pub struct TailEvent {
    pub outcome: String,
    pub timestamp_ms: u64,
    pub script_name: Option<String>,
    /// "GET /path 200", "cron 0 * * * *", "queue my-queue (10)", ...
    pub summary: String,
    pub logs: Vec<(String, String)>, // (level, message)
    pub exceptions: Vec<String>,
    pub cpu_ms: Option<u64>,
    pub wall_ms: Option<u64>,
    pub raw: Value,
}

pub fn decode_event(v: &Value) -> TailEvent {
    let outcome = v
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let timestamp_ms = v.get("eventTimestamp").and_then(Value::as_u64).unwrap_or(0);
    let script_name = v
        .get("scriptName")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ev = v.get("event").cloned().unwrap_or(Value::Null);
    let summary = if let Some(req) = ev.get("request") {
        let method = req.get("method").and_then(Value::as_str).unwrap_or("?");
        let url = req.get("url").and_then(Value::as_str).unwrap_or("");
        let path = url
            .splitn(4, '/')
            .nth(3)
            .map(|p| format!("/{p}"))
            .unwrap_or_else(|| url.to_string());
        let status = ev
            .pointer("/response/status")
            .and_then(Value::as_u64)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".into());
        format!("{method} {path} {status}")
    } else if let Some(cron) = ev.get("cron").and_then(Value::as_str) {
        format!("cron {cron}")
    } else if let Some(q) = ev.get("queue").and_then(Value::as_str) {
        let n = ev.get("batchSize").and_then(Value::as_u64).unwrap_or(0);
        format!("queue {q} ({n})")
    } else if ev.get("scheduledTime").is_some() {
        "alarm".to_string()
    } else if let Some(m) = ev.get("rpcMethod").and_then(Value::as_str) {
        format!("rpc {m}")
    } else if let Some(t) = ev.get("type").and_then(Value::as_str) {
        format!(
            "tail-info {t}: {}",
            ev.get("message").and_then(Value::as_str).unwrap_or("")
        )
    } else if ev.is_null() {
        "event".to_string()
    } else {
        truncate(&ev.to_string(), 60)
    };
    let logs = v
        .get("logs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|l| {
                    let level = l
                        .get("level")
                        .and_then(Value::as_str)
                        .unwrap_or("log")
                        .to_string();
                    let msg = match l.get("message") {
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .map(|p| match p {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(" "),
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    (level, msg)
                })
                .collect()
        })
        .unwrap_or_default();
    let exceptions = v
        .get("exceptions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| {
                    format!(
                        "{}: {}",
                        e.get("name").and_then(Value::as_str).unwrap_or("Error"),
                        e.get("message").and_then(Value::as_str).unwrap_or("")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    TailEvent {
        outcome,
        timestamp_ms,
        script_name,
        summary,
        logs,
        exceptions,
        cpu_ms: v.get("cpuTime").and_then(Value::as_u64),
        wall_ms: v.get("wallTime").and_then(Value::as_u64),
        raw: v.clone(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

#[allow(clippy::large_enum_variant)]
pub enum TailMsg {
    Event(TailEvent),
    Info(String),
    Closed(String),
}

/// Connect and pump events until `stop` resolves or the socket closes.
pub async fn run(
    url: String,
    tx: mpsc::Sender<TailMsg>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let mut req = match url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(TailMsg::Closed(format!("bad tail url: {e}"))).await;
            return;
        }
    };
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "trace-v1".parse().expect("static header"),
    );
    req.headers_mut().insert(
        "User-Agent",
        concat!("flaretop/", env!("CARGO_PKG_VERSION"))
            .parse()
            .expect("static header"),
    );
    let (ws, _) = match tokio_tungstenite::connect_async(req).await {
        Ok(x) => x,
        Err(e) => {
            let _ = tx
                .send(TailMsg::Closed(format!("connect failed: {e}")))
                .await;
            return;
        }
    };
    let (mut sink, mut stream) = ws.split();
    if sink
        .send(Message::Text(r#"{"debug":false}"#.into()))
        .await
        .is_err()
    {
        let _ = tx.send(TailMsg::Closed("handshake failed".into())).await;
        return;
    }
    let _ = tx.send(TailMsg::Info("connected".into())).await;
    let mut ping = tokio::time::interval(std::time::Duration::from_secs(10));
    ping.tick().await;
    loop {
        tokio::select! {
            _ = &mut stop => {
                let _ = sink.send(Message::Close(None)).await;
                let _ = tx.send(TailMsg::Closed("stopped".into())).await;
                return;
            }
            _ = ping.tick() => {
                if sink.send(Message::Ping(b"flaretop tail ping".to_vec().into())).await.is_err() {
                    let _ = tx.send(TailMsg::Closed("ping failed".into())).await;
                    return;
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(msg @ (Message::Text(_) | Message::Binary(_)))) => {
                        // The backend sends events as binary frames holding JSON.
                        let t: String = match msg {
                            Message::Text(t) => t.to_string(),
                            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                            _ => unreachable!(),
                        };
                        match serde_json::from_str::<Value>(&t) {
                            Ok(v) => {
                                if tx.send(TailMsg::Event(decode_event(&v))).await.is_err() {
                                    return;
                                }
                            }
                            Err(_) => {
                                let _ = tx.send(TailMsg::Info(truncate(&t, 120))).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = tx.send(TailMsg::Closed("server closed the tail".into())).await;
                        return;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        let _ = tx.send(TailMsg::Closed(format!("socket error: {e}"))).await;
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_request_event() {
        let v = json!({
            "outcome": "ok", "eventTimestamp": 1700000000000u64, "scriptName": "api",
            "event": { "request": { "method": "GET", "url": "https://x.dev/a/b?c=1" }, "response": { "status": 200 } },
            "logs": [{ "level": "info", "message": ["hello", 42] }],
            "exceptions": []
        });
        let e = decode_event(&v);
        assert_eq!(e.summary, "GET /a/b?c=1 200");
        assert_eq!(e.logs[0].1, "hello 42");
    }

    #[test]
    fn filters_expand_error() {
        let f = TailFilters {
            status: vec!["error".into()],
            ..Default::default()
        };
        let j = f.to_json();
        assert_eq!(j[0]["outcome"].as_array().unwrap().len(), 4);
    }
}
