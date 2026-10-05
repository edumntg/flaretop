//! Workers scripts: list, settings, cron, secrets, subdomain, domains,
//! versions, deployments. Notes that matter for callers:
//! - `/workers/scripts` returns everything in one page, no `result_info`.
//! - secret values are never readable; only names.
//! - rollback is "deploy the older version at 100%".

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Body, Client};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Script {
    pub id: String,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub modified_on: Option<String>,
    #[serde(default)]
    pub handlers: Vec<String>,
    #[serde(default)]
    pub usage_model: Option<String>,
    #[serde(default)]
    pub compatibility_date: Option<String>,
    #[serde(default)]
    pub compatibility_flags: Vec<String>,
    #[serde(default)]
    pub last_deployed_from: Option<String>,
    #[serde(default)]
    pub has_assets: Option<bool>,
    #[serde(default)]
    pub has_modules: Option<bool>,
    #[serde(default)]
    pub logpush: Option<bool>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub observability: Option<Value>,
    #[serde(default)]
    pub placement: Option<Value>,
}

pub async fn list(c: &Client, account: &str) -> ApiResult<Vec<Script>> {
    let mut v: Vec<Script> = c
        .get(&format!("/accounts/{account}/workers/scripts"), &[])
        .await?;
    v.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(v)
}

/// Full settings object: bindings, compat, limits, observability, tail consumers.
pub async fn settings(c: &Client, account: &str, name: &str) -> ApiResult<Value> {
    c.get(
        &format!("/accounts/{account}/workers/scripts/{name}/settings"),
        &[],
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Schedule {
    pub cron: String,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub modified_on: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SchedulesWrap {
    #[serde(default)]
    schedules: Vec<Schedule>,
}

pub async fn schedules(c: &Client, account: &str, name: &str) -> ApiResult<Vec<Schedule>> {
    let w: SchedulesWrap = c
        .get(
            &format!("/accounts/{account}/workers/scripts/{name}/schedules"),
            &[],
        )
        .await?;
    Ok(w.schedules)
}

/// Replaces the whole cron set.
pub async fn put_schedules(
    c: &Client,
    account: &str,
    name: &str,
    crons: &[String],
) -> ApiResult<Value> {
    let body: Vec<Value> = crons.iter().map(|cr| json!({ "cron": cr })).collect();
    c.put(
        &format!("/accounts/{account}/workers/scripts/{name}/schedules"),
        Value::Array(body),
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Secret {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

pub async fn secrets(c: &Client, account: &str, name: &str) -> ApiResult<Vec<Secret>> {
    c.get(
        &format!("/accounts/{account}/workers/scripts/{name}/secrets"),
        &[],
    )
    .await
}

pub async fn put_secret(
    c: &Client,
    account: &str,
    name: &str,
    key: &str,
    value: &str,
) -> ApiResult<Value> {
    c.put(
        &format!("/accounts/{account}/workers/scripts/{name}/secrets"),
        json!({ "name": key, "text": value, "type": "secret_text" }),
    )
    .await
}

pub async fn delete_secret(c: &Client, account: &str, name: &str, key: &str) -> ApiResult<Value> {
    c.delete(&format!(
        "/accounts/{account}/workers/scripts/{name}/secrets/{}",
        crate::api::encode_segment(key)
    ))
    .await
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Subdomain {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub previews_enabled: bool,
}

pub async fn subdomain(c: &Client, account: &str, name: &str) -> ApiResult<Subdomain> {
    c.get(
        &format!("/accounts/{account}/workers/scripts/{name}/subdomain"),
        &[],
    )
    .await
}

pub async fn set_subdomain(
    c: &Client,
    account: &str,
    name: &str,
    enabled: bool,
    previews: bool,
) -> ApiResult<Subdomain> {
    c.post(
        &format!("/accounts/{account}/workers/scripts/{name}/subdomain"),
        json!({ "enabled": enabled, "previews_enabled": previews }),
    )
    .await
}

#[derive(Debug, Deserialize)]
struct AccountSubdomain {
    subdomain: String,
}

/// The account's `<name>.workers.dev` label.
pub async fn account_subdomain(c: &Client, account: &str) -> ApiResult<String> {
    let s: AccountSubdomain = c
        .get(&format!("/accounts/{account}/workers/subdomain"), &[])
        .await?;
    Ok(s.subdomain)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Domain {
    pub id: String,
    pub hostname: String,
    pub service: String,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub zone_id: Option<String>,
    #[serde(default)]
    pub zone_name: Option<String>,
}

pub async fn domains(c: &Client, account: &str) -> ApiResult<Vec<Domain>> {
    c.get(&format!("/accounts/{account}/workers/domains"), &[])
        .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub id: String,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub annotations: Option<Value>,
}

impl Version {
    pub fn created_on(&self) -> Option<&str> {
        self.metadata.as_ref()?.get("created_on")?.as_str()
    }
    pub fn source(&self) -> Option<&str> {
        self.metadata.as_ref()?.get("source")?.as_str()
    }
    pub fn author(&self) -> Option<&str> {
        self.metadata.as_ref()?.get("author_email")?.as_str()
    }
    pub fn message(&self) -> Option<&str> {
        self.annotations.as_ref()?.get("workers/message")?.as_str()
    }
    pub fn triggered_by(&self) -> Option<&str> {
        self.annotations
            .as_ref()?
            .get("workers/triggered_by")?
            .as_str()
    }
    pub fn tag(&self) -> Option<&str> {
        self.annotations.as_ref()?.get("workers/tag")?.as_str()
    }
}

#[derive(Debug, Deserialize)]
struct VersionsWrap {
    #[serde(default)]
    items: Vec<Version>,
}

pub async fn versions(c: &Client, account: &str, name: &str) -> ApiResult<Vec<Version>> {
    let w: VersionsWrap = c
        .get(
            &format!("/accounts/{account}/workers/scripts/{name}/versions"),
            &[("per_page", "50".into())],
        )
        .await?;
    Ok(w.items)
}

pub async fn version(c: &Client, account: &str, name: &str, version_id: &str) -> ApiResult<Value> {
    c.get(
        &format!("/accounts/{account}/workers/scripts/{name}/versions/{version_id}"),
        &[],
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentVersion {
    pub version_id: String,
    pub percentage: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    pub id: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub author_email: Option<String>,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub annotations: Option<Value>,
    #[serde(default)]
    pub versions: Vec<DeploymentVersion>,
}

impl Deployment {
    pub fn message(&self) -> Option<&str> {
        self.annotations.as_ref()?.get("workers/message")?.as_str()
    }
    pub fn triggered_by(&self) -> Option<&str> {
        self.annotations
            .as_ref()?
            .get("workers/triggered_by")?
            .as_str()
    }
}

#[derive(Debug, Deserialize)]
struct DeploymentsWrap {
    #[serde(default)]
    deployments: Vec<Deployment>,
}

pub async fn deployments(c: &Client, account: &str, name: &str) -> ApiResult<Vec<Deployment>> {
    let w: DeploymentsWrap = c
        .get(
            &format!("/accounts/{account}/workers/scripts/{name}/deployments"),
            &[],
        )
        .await?;
    Ok(w.deployments)
}

/// Gradual rollout or rollback: `versions` percentages must sum to 100.
pub async fn create_deployment(
    c: &Client,
    account: &str,
    name: &str,
    versions: &[(String, f64)],
    message: Option<&str>,
) -> ApiResult<Value> {
    let vs: Vec<Value> = versions
        .iter()
        .map(|(id, pct)| json!({ "version_id": id, "percentage": pct }))
        .collect();
    let mut body = json!({ "strategy": "percentage", "versions": vs });
    if let Some(m) = message {
        body["annotations"] = json!({ "workers/message": m });
    }
    c.request(
        reqwest::Method::POST,
        &format!("/accounts/{account}/workers/scripts/{name}/deployments"),
        &[("force", "true".into())],
        Body::Json(body),
    )
    .await
}

pub async fn delete_script(c: &Client, account: &str, name: &str, force: bool) -> ApiResult<Value> {
    c.request(
        reqwest::Method::DELETE,
        &format!("/accounts/{account}/workers/scripts/{name}"),
        &[("force", force.to_string())],
        Body::None,
    )
    .await
}

/// Download the deployed bundle. Modules come back as multipart/form-data;
/// a service-worker script comes back as a single JS body. We return the raw
/// text plus the content type and let the UI split parts for display.
pub async fn content(c: &Client, account: &str, name: &str) -> ApiResult<(String, String)> {
    let (bytes, headers) = c
        .get_bytes(
            &format!("/accounts/{account}/workers/scripts/{name}/content/v2"),
            &[],
            &[],
        )
        .await?;
    let ct = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok((String::from_utf8_lossy(&bytes).to_string(), ct))
}

/// Split a multipart body into (part name, content) pairs for display.
pub fn split_multipart(body: &str, content_type: &str) -> Vec<(String, String)> {
    let Some(boundary) = content_type
        .split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("boundary="))
    else {
        return vec![("script".into(), body.to_string())];
    };
    let boundary = boundary.trim_matches('"');
    let delim = format!("--{boundary}");
    let mut out = Vec::new();
    for part in body.split(&delim) {
        let part = part.trim_start_matches("\r\n").trim_start_matches('\n');
        if part.is_empty() || part.starts_with("--") {
            continue;
        }
        let (head, content) = match part.find("\r\n\r\n") {
            Some(i) => (&part[..i], &part[i + 4..]),
            None => match part.find("\n\n") {
                Some(i) => (&part[..i], &part[i + 2..]),
                None => continue,
            },
        };
        let name = head
            .lines()
            .find_map(|l| {
                let l = l.trim();
                if l.to_ascii_lowercase().starts_with("content-disposition") {
                    l.split(';').map(str::trim).find_map(|kv| {
                        kv.strip_prefix("name=")
                            .map(|v| v.trim_matches('"').to_string())
                    })
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "part".into());
        let content = content.trim_end_matches("\r\n").trim_end_matches('\n');
        out.push((name, content.to_string()));
    }
    out
}

/// Metrics for the last 24h from GraphQL (requests, errors, p50/p99 CPU).
#[derive(Debug, Clone, Default)]
pub struct InvocationStats {
    pub requests: u64,
    pub errors: u64,
    pub subrequests: u64,
    pub cpu_p50_us: f64,
    pub cpu_p99_us: f64,
}

pub async fn invocation_stats_24h(
    c: &Client,
    account: &str,
    name: &str,
) -> ApiResult<InvocationStats> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let start = crate::auth::civil_from_unix(now - 86_400) + "Z";
    let end = crate::auth::civil_from_unix(now) + "Z";
    let q = r#"query($account: String!, $start: Time!, $end: Time!, $name: String!) {
  viewer { accounts(filter: {accountTag: $account}) {
    workersInvocationsAdaptive(limit: 1, filter: {scriptName: $name, datetime_geq: $start, datetime_leq: $end}) {
      sum { requests errors subrequests }
      quantiles { cpuTimeP50 cpuTimeP99 }
    } } } }"#;
    let data = c
        .graphql(
            q,
            json!({ "account": account, "start": start, "end": end, "name": name }),
        )
        .await?;
    let row = data
        .pointer("/viewer/accounts/0/workersInvocationsAdaptive/0")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(InvocationStats {
        requests: row
            .pointer("/sum/requests")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        errors: row
            .pointer("/sum/errors")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        subrequests: row
            .pointer("/sum/subrequests")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cpu_p50_us: row
            .pointer("/quantiles/cpuTimeP50")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        cpu_p99_us: row
            .pointer("/quantiles/cpuTimeP99")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_multipart_bundle() {
        let ct = "multipart/form-data; boundary=abc";
        let body = "--abc\r\nContent-Disposition: form-data; name=\"index.js\"; filename=\"index.js\"\r\nContent-Type: application/javascript+module\r\n\r\nexport default {}\r\n--abc\r\nContent-Disposition: form-data; name=\"a.wasm\"\r\n\r\nBIN\r\n--abc--\r\n";
        let parts = split_multipart(body, ct);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].0, "index.js");
        assert_eq!(parts[0].1, "export default {}");
        assert_eq!(parts[1].0, "a.wasm");
    }

    #[test]
    fn plain_body_is_one_part() {
        let parts = split_multipart(
            "addEventListener('fetch', () => {})",
            "application/javascript",
        );
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].0, "script");
    }
}
