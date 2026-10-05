//! Workers KV: namespaces, cursor-paginated keys (limit 10–1000), values,
//! metadata, bulk get (≤100) and bulk delete. There is no exact key count.

use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Body, Client, Page, encode_segment};

#[derive(Debug, Clone, Deserialize)]
pub struct Namespace {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub supports_url_encoding: Option<bool>,
}

pub async fn namespaces(c: &Client, account: &str) -> ApiResult<Vec<Namespace>> {
    c.get_all_pages(
        &format!("/accounts/{account}/storage/kv/namespaces"),
        100,
        &[("order", "title".into()), ("direction", "asc".into())],
    )
    .await
}

pub async fn create_namespace(c: &Client, account: &str, title: &str) -> ApiResult<Namespace> {
    c.post(
        &format!("/accounts/{account}/storage/kv/namespaces"),
        json!({ "title": title }),
    )
    .await
}

pub async fn rename_namespace(
    c: &Client,
    account: &str,
    id: &str,
    title: &str,
) -> ApiResult<Value> {
    c.put(
        &format!("/accounts/{account}/storage/kv/namespaces/{id}"),
        json!({ "title": title }),
    )
    .await
}

pub async fn delete_namespace(c: &Client, account: &str, id: &str) -> ApiResult<Value> {
    c.delete(&format!("/accounts/{account}/storage/kv/namespaces/{id}"))
        .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Key {
    pub name: String,
    #[serde(default)]
    pub expiration: Option<u64>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

pub struct KeyPage {
    pub keys: Vec<Key>,
    pub cursor: Option<String>,
}

pub async fn keys(
    c: &Client,
    account: &str,
    ns: &str,
    prefix: &str,
    cursor: Option<&str>,
    limit: usize,
) -> ApiResult<KeyPage> {
    let mut q: Vec<(&str, String)> = vec![("limit", limit.clamp(10, 1000).to_string())];
    if !prefix.is_empty() {
        q.push(("prefix", prefix.to_string()));
    }
    if let Some(cur) = cursor {
        q.push(("cursor", cur.to_string()));
    }
    let p: Page<Vec<Key>> = c
        .get_page(
            &format!("/accounts/{account}/storage/kv/namespaces/{ns}/keys"),
            &q,
        )
        .await?;
    Ok(KeyPage {
        cursor: p.cursor(),
        keys: p.result,
    })
}

pub async fn get_value(
    c: &Client,
    account: &str,
    ns: &str,
    key: &str,
) -> ApiResult<(Bytes, Option<u64>)> {
    let (bytes, headers) = c
        .get_bytes(
            &format!(
                "/accounts/{account}/storage/kv/namespaces/{ns}/values/{}",
                encode_segment(key)
            ),
            &[],
            &[],
        )
        .await?;
    let exp = headers
        .get("expiration")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    Ok((bytes, exp))
}

pub async fn metadata(c: &Client, account: &str, ns: &str, key: &str) -> ApiResult<Value> {
    c.get(
        &format!(
            "/accounts/{account}/storage/kv/namespaces/{ns}/metadata/{}",
            encode_segment(key)
        ),
        &[],
    )
    .await
}

/// Put a value. `metadata` is JSON; `ttl` is seconds (≥ 60).
pub async fn put_value(
    c: &Client,
    account: &str,
    ns: &str,
    key: &str,
    value: Vec<u8>,
    metadata: Option<Value>,
    ttl: Option<u64>,
) -> ApiResult<Value> {
    let mut q: Vec<(&str, String)> = Vec::new();
    if let Some(t) = ttl {
        q.push(("expiration_ttl", t.to_string()));
    }
    let path = format!(
        "/accounts/{account}/storage/kv/namespaces/{ns}/values/{}",
        encode_segment(key)
    );
    let body = match metadata {
        Some(m) => {
            let form = reqwest::multipart::Form::new()
                .part("value", reqwest::multipart::Part::bytes(value))
                .text("metadata", m.to_string());
            Body::Multipart(form)
        }
        None => Body::Bytes(Bytes::from(value), "application/octet-stream"),
    };
    c.send_body(reqwest::Method::PUT, &path, &q, &[], body)
        .await
}

pub async fn delete_key(c: &Client, account: &str, ns: &str, key: &str) -> ApiResult<Value> {
    c.delete(&format!(
        "/accounts/{account}/storage/kv/namespaces/{ns}/values/{}",
        encode_segment(key)
    ))
    .await
}

/// Delete up to 10,000 keys in one call.
pub async fn bulk_delete(c: &Client, account: &str, ns: &str, keys: &[String]) -> ApiResult<Value> {
    c.post(
        &format!("/accounts/{account}/storage/kv/namespaces/{ns}/bulk/delete"),
        json!(keys),
    )
    .await
}

/// Fetch up to 100 values as text in one call. Missing keys map to null.
pub async fn bulk_get(c: &Client, account: &str, ns: &str, keys: &[String]) -> ApiResult<Value> {
    let v: Value = c
        .post(
            &format!("/accounts/{account}/storage/kv/namespaces/{ns}/bulk/get"),
            json!({ "keys": keys, "withMetadata": false }),
        )
        .await?;
    Ok(v.get("values").cloned().unwrap_or(Value::Null))
}
