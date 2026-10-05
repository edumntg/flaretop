//! Queues: CRUD, consumers, push, purge, pause, and the HTTP pull API.
//! There is no non-consuming peek: a pull leases messages and a "preview"
//! via pull + retry costs an attempt.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Client};

#[derive(Debug, Clone, Deserialize)]
pub struct Consumer {
    #[serde(default)]
    pub consumer_id: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub dead_letter_queue: Option<String>,
    #[serde(default)]
    pub settings: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Producer {
    #[serde(default)]
    pub script: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Queue {
    pub queue_id: String,
    pub queue_name: String,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub modified_on: Option<String>,
    #[serde(default)]
    pub settings: Option<Value>,
    #[serde(default)]
    pub producers: Vec<Producer>,
    #[serde(default)]
    pub consumers: Vec<Consumer>,
}

impl Queue {
    pub fn paused(&self) -> bool {
        self.settings
            .as_ref()
            .and_then(|s| s.get("delivery_paused"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn delivery_delay(&self) -> u64 {
        self.settings
            .as_ref()
            .and_then(|s| s.get("delivery_delay"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }
    pub fn retention(&self) -> u64 {
        self.settings
            .as_ref()
            .and_then(|s| s.get("message_retention_period"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }
    pub fn has_pull_consumer(&self) -> bool {
        self.consumers
            .iter()
            .any(|c| c.kind.as_deref() == Some("http_pull"))
    }
}

pub async fn list(c: &Client, account: &str) -> ApiResult<Vec<Queue>> {
    let mut v: Vec<Queue> = c
        .get_all_pages(&format!("/accounts/{account}/queues"), 100, &[])
        .await?;
    v.sort_by(|a, b| a.queue_name.cmp(&b.queue_name));
    Ok(v)
}

pub async fn get(c: &Client, account: &str, id: &str) -> ApiResult<Queue> {
    c.get(&format!("/accounts/{account}/queues/{id}"), &[])
        .await
}

pub async fn create(c: &Client, account: &str, name: &str) -> ApiResult<Queue> {
    c.post(
        &format!("/accounts/{account}/queues"),
        json!({ "queue_name": name }),
    )
    .await
}

pub async fn delete(c: &Client, account: &str, id: &str) -> ApiResult<Value> {
    c.delete(&format!("/accounts/{account}/queues/{id}")).await
}

pub async fn set_paused(c: &Client, account: &str, id: &str, paused: bool) -> ApiResult<Queue> {
    c.patch(
        &format!("/accounts/{account}/queues/{id}"),
        json!({ "settings": { "delivery_paused": paused } }),
    )
    .await
}

pub async fn push(
    c: &Client,
    account: &str,
    id: &str,
    body: &str,
    json_body: bool,
    delay: Option<u64>,
) -> ApiResult<Value> {
    let mut msg = if json_body {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::String(body.to_string()));
        json!({ "body": v, "content_type": "json" })
    } else {
        json!({ "body": body, "content_type": "text" })
    };
    if let Some(d) = delay {
        msg["delay_seconds"] = json!(d);
    }
    c.post(&format!("/accounts/{account}/queues/{id}/messages"), msg)
        .await
}

pub async fn purge(c: &Client, account: &str, id: &str) -> ApiResult<Value> {
    c.post(
        &format!("/accounts/{account}/queues/{id}/purge"),
        json!({ "delete_messages_permanently": true }),
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct PulledMessage {
    pub id: String,
    pub lease_id: String,
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PullResult {
    #[serde(default)]
    pub message_backlog_count: u64,
    #[serde(default)]
    pub messages: Vec<PulledMessage>,
}

/// Requires an `http_pull` consumer on the queue.
pub async fn pull(
    c: &Client,
    account: &str,
    id: &str,
    batch: usize,
    visibility_ms: u64,
) -> ApiResult<PullResult> {
    c.post(
        &format!("/accounts/{account}/queues/{id}/messages/pull"),
        json!({ "batch_size": batch.clamp(1, 100), "visibility_timeout_ms": visibility_ms }),
    )
    .await
}

pub async fn ack(
    c: &Client,
    account: &str,
    id: &str,
    acks: &[String],
    retries: &[String],
) -> ApiResult<Value> {
    let a: Vec<Value> = acks.iter().map(|l| json!({ "lease_id": l })).collect();
    let r: Vec<Value> = retries
        .iter()
        .map(|l| json!({ "lease_id": l, "delay_seconds": 0 }))
        .collect();
    c.post(
        &format!("/accounts/{account}/queues/{id}/messages/ack"),
        json!({ "acks": a, "retries": r }),
    )
    .await
}

/// Backlog (messages, bytes) averaged over the last hour from GraphQL.
pub async fn backlog(c: &Client, account: &str, queue_id: &str) -> ApiResult<(f64, f64)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let start = crate::auth::civil_from_unix(now - 3600) + "Z";
    let end = crate::auth::civil_from_unix(now) + "Z";
    let q = r#"query($account: String!, $start: Time!, $end: Time!, $queue: String!) {
  viewer { accounts(filter: {accountTag: $account}) {
    queueBacklogAdaptiveGroups(limit: 1, filter: {datetime_geq: $start, datetime_leq: $end, queueId: $queue}) {
      avg { messages bytes }
    } } } }"#;
    let data = c
        .graphql(
            q,
            json!({ "account": account, "start": start, "end": end, "queue": queue_id }),
        )
        .await?;
    let row = data
        .pointer("/viewer/accounts/0/queueBacklogAdaptiveGroups/0/avg")
        .cloned()
        .unwrap_or(Value::Null);
    Ok((
        row.get("messages").and_then(Value::as_f64).unwrap_or(0.0),
        row.get("bytes").and_then(Value::as_f64).unwrap_or(0.0),
    ))
}
