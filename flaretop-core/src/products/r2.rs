//! R2 over the Cloudflare REST API: buckets and the (now public) object
//! routes. Objects over 300 MB and multipart uploads need the S3 API.

use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Body, Client, Page, encode_segment};

#[derive(Debug, Clone, Deserialize)]
pub struct Bucket {
    pub name: String,
    #[serde(default)]
    pub creation_date: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BucketsWrap {
    #[serde(default)]
    buckets: Vec<Bucket>,
}

pub async fn buckets(c: &Client, account: &str) -> ApiResult<Vec<Bucket>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut q: Vec<(&str, String)> = vec![("per_page", "1000".into())];
        if let Some(cur) = &cursor {
            q.push(("cursor", cur.clone()));
        }
        let p: Page<BucketsWrap> = c
            .get_page(&format!("/accounts/{account}/r2/buckets"), &q)
            .await?;
        cursor = p.cursor();
        out.extend(p.result.buckets);
        if cursor.is_none() {
            break;
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub async fn get_bucket(c: &Client, account: &str, name: &str) -> ApiResult<Bucket> {
    c.get(&format!("/accounts/{account}/r2/buckets/{name}"), &[])
        .await
}

pub async fn create_bucket(
    c: &Client,
    account: &str,
    name: &str,
    location_hint: Option<&str>,
) -> ApiResult<Bucket> {
    let mut body = json!({ "name": name });
    if let Some(l) = location_hint {
        body["locationHint"] = json!(l);
    }
    c.post(&format!("/accounts/{account}/r2/buckets"), body)
        .await
}

/// Bucket must be empty.
pub async fn delete_bucket(c: &Client, account: &str, name: &str) -> ApiResult<Value> {
    c.delete(&format!("/accounts/{account}/r2/buckets/{name}"))
        .await
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ManagedDomain {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(rename = "bucketId", default)]
    pub bucket_id: Option<String>,
}

pub async fn managed_domain(c: &Client, account: &str, bucket: &str) -> ApiResult<ManagedDomain> {
    c.get(
        &format!("/accounts/{account}/r2/buckets/{bucket}/domains/managed"),
        &[],
    )
    .await
}

pub async fn set_managed_domain(
    c: &Client,
    account: &str,
    bucket: &str,
    enabled: bool,
) -> ApiResult<ManagedDomain> {
    c.put(
        &format!("/accounts/{account}/r2/buckets/{bucket}/domains/managed"),
        json!({ "enabled": enabled }),
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomDomain {
    pub domain: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "zoneName")]
    pub zone_name: Option<String>,
    #[serde(default)]
    pub status: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DomainsWrap {
    #[serde(default)]
    domains: Vec<CustomDomain>,
}

pub async fn custom_domains(
    c: &Client,
    account: &str,
    bucket: &str,
) -> ApiResult<Vec<CustomDomain>> {
    let w: DomainsWrap = c
        .get(
            &format!("/accounts/{account}/r2/buckets/{bucket}/domains/custom"),
            &[],
        )
        .await?;
    Ok(w.domains)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Object {
    pub key: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    pub http_metadata: Option<Value>,
    #[serde(default)]
    pub custom_metadata: Option<Value>,
}

impl Object {
    pub fn content_type(&self) -> Option<&str> {
        self.http_metadata.as_ref()?.get("contentType")?.as_str()
    }
}

pub struct ObjectPage {
    pub objects: Vec<Object>,
    /// Common prefixes ("folders") when a delimiter was used.
    pub folders: Vec<String>,
    pub cursor: Option<String>,
    pub truncated: bool,
}

/// List with `delimiter=/` for a folder view. `result` is defensively parsed
/// because the API has returned it as an array, `{objects:[]}` and a map.
pub async fn objects(
    c: &Client,
    account: &str,
    bucket: &str,
    prefix: &str,
    delimiter: Option<&str>,
    cursor: Option<&str>,
    per_page: usize,
) -> ApiResult<ObjectPage> {
    let mut q: Vec<(&str, String)> = vec![("per_page", per_page.clamp(1, 1000).to_string())];
    if !prefix.is_empty() {
        q.push(("prefix", prefix.to_string()));
    }
    if let Some(d) = delimiter {
        q.push(("delimiter", d.to_string()));
    }
    if let Some(cur) = cursor {
        q.push(("cursor", cur.to_string()));
    }
    let p: Page<Value> = c
        .get_page(
            &format!("/accounts/{account}/r2/buckets/{bucket}/objects"),
            &q,
        )
        .await?;
    let raw_objects: Vec<Value> = match &p.result {
        Value::Array(a) => a.clone(),
        Value::Object(m) => m
            .get("objects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let objects: Vec<Object> = raw_objects
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    let info = p.result_info.clone().unwrap_or(Value::Null);
    let folders: Vec<String> = info
        .get("delimited")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let truncated = info
        .get("is_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ObjectPage {
        objects,
        folders,
        cursor: if truncated { p.cursor() } else { None },
        truncated,
    })
}

pub async fn get_object(
    c: &Client,
    account: &str,
    bucket: &str,
    key: &str,
) -> ApiResult<(Bytes, Option<String>)> {
    let (bytes, headers) = c
        .get_bytes(
            &format!(
                "/accounts/{account}/r2/buckets/{bucket}/objects/{}",
                encode_key(key)
            ),
            &[],
            &[],
        )
        .await?;
    let ct = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Ok((bytes, ct))
}

/// Up to 300 MB through this route.
pub async fn put_object(
    c: &Client,
    account: &str,
    bucket: &str,
    key: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> ApiResult<Value> {
    let ct: &'static str = Box::leak(content_type.to_string().into_boxed_str());
    c.send_body(
        reqwest::Method::PUT,
        &format!(
            "/accounts/{account}/r2/buckets/{bucket}/objects/{}",
            encode_key(key)
        ),
        &[],
        &[],
        Body::Bytes(Bytes::from(bytes), ct),
    )
    .await
}

pub async fn delete_object(c: &Client, account: &str, bucket: &str, key: &str) -> ApiResult<Value> {
    c.delete(&format!(
        "/accounts/{account}/r2/buckets/{bucket}/objects/{}",
        encode_key(key)
    ))
    .await
}

/// 1–1000 keys per call.
pub async fn delete_objects(
    c: &Client,
    account: &str,
    bucket: &str,
    keys: &[String],
) -> ApiResult<Value> {
    c.delete_json(
        &format!("/accounts/{account}/r2/buckets/{bucket}/objects"),
        json!(keys),
    )
    .await
}

/// Per-bucket size and count from GraphQL (last 24h snapshot).
#[derive(Debug, Clone, Default)]
pub struct BucketStats {
    pub object_count: u64,
    pub payload_size: u64,
}

pub async fn bucket_stats(c: &Client, account: &str, bucket: &str) -> ApiResult<BucketStats> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let start = crate::auth::civil_from_unix(now - 86_400) + "Z";
    let end = crate::auth::civil_from_unix(now) + "Z";
    let q = r#"query($account: String!, $start: Time!, $end: Time!, $bucket: String!) {
  viewer { accounts(filter: {accountTag: $account}) {
    r2StorageAdaptiveGroups(limit: 1, filter: {datetime_geq: $start, datetime_leq: $end, bucketName: $bucket}) {
      max { objectCount payloadSize }
    } } } }"#;
    let data = c
        .graphql(
            q,
            json!({ "account": account, "start": start, "end": end, "bucket": bucket }),
        )
        .await?;
    let row = data
        .pointer("/viewer/accounts/0/r2StorageAdaptiveGroups/0/max")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(BucketStats {
        object_count: row.get("objectCount").and_then(Value::as_u64).unwrap_or(0),
        payload_size: row.get("payloadSize").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Object keys may contain slashes that must stay as path separators.
fn encode_key(key: &str) -> String {
    key.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_encoding_keeps_slashes() {
        assert_eq!(encode_key("a/b c/d#e"), "a/b%20c/d%23e");
    }
}
