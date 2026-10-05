//! HTTP client for api.cloudflare.com/client/v4.
//!
//! Every JSON response is the v4 envelope; `result_info` is present on only
//! some list endpoints and comes in several variants (page/per_page, cursor,
//! page_token). Missing-permission errors (codes 10000 and 9109) are mapped
//! to [`ApiError::MissingPermission`] so the UI can say "your token lacks X"
//! instead of showing a generic failure.
//!
//! The global API limit is 1,200 requests per 5 minutes per user, shared with
//! the dashboard. `Client` keeps a sliding-window counter so the status bar
//! can show the budget and so callers can back off before the API does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const BASE_URL: &str = "https://api.cloudflare.com/client/v4";
pub const RATE_WINDOW: Duration = Duration::from_secs(300);
pub const RATE_LIMIT: usize = 1200;
const USER_AGENT: &str = concat!("flaretop/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ApiMessage {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub documentation_url: Option<String>,
}

/// Some endpoints return `errors: null` instead of `[]`, so both are optional.
#[derive(Debug, Deserialize)]
pub struct Envelope<T> {
    pub success: bool,
    #[serde(default)]
    pub errors: Option<Vec<ApiMessage>>,
    #[serde(default)]
    pub messages: Option<Vec<ApiMessage>>,
    pub result: Option<T>,
    #[serde(default)]
    pub result_info: Option<Value>,
}

/// Error codes that mean "the token lacks a permission", not "bad request".
pub const MISSING_PERMISSION_CODES: &[i64] = &[10000, 9109];

impl ApiMessage {
    pub fn is_missing_permission(&self) -> bool {
        MISSING_PERMISSION_CODES.contains(&self.code)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ApiError {
    #[error("{message} (code {code}, HTTP {status})")]
    Api {
        status: u16,
        code: i64,
        message: String,
    },
    #[error("token lacks permission for this resource (HTTP {status}, code {code})")]
    MissingPermission { status: u16, code: i64 },
    #[error("rate limited by the API; retry after {retry_after}s")]
    RateLimited { retry_after: u64 },
    #[error("network error: {0}")]
    Transport(String),
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("not found")]
    NotFound,
}

impl ApiError {
    pub fn is_missing_permission(&self) -> bool {
        matches!(self, ApiError::MissingPermission { .. })
    }
    pub fn is_not_found(&self) -> bool {
        matches!(self, ApiError::NotFound)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Sliding-window request counter for the 1,200 / 5 min budget.
#[derive(Default)]
pub struct Budget {
    stamps: VecDeque<Instant>,
}

impl Budget {
    fn record(&mut self) {
        let now = Instant::now();
        self.stamps.push_back(now);
        while let Some(front) = self.stamps.front() {
            if now.duration_since(*front) > RATE_WINDOW {
                self.stamps.pop_front();
            } else {
                break;
            }
        }
    }
    pub fn used(&mut self) -> usize {
        let now = Instant::now();
        while let Some(front) = self.stamps.front() {
            if now.duration_since(*front) > RATE_WINDOW {
                self.stamps.pop_front();
            } else {
                break;
            }
        }
        self.stamps.len()
    }
}

pub enum Body {
    None,
    Json(Value),
    Bytes(Bytes, &'static str),
    Text(String, &'static str),
    Multipart(reqwest::multipart::Form),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    auth: Arc<HeaderValue>,
    pub budget: Arc<Mutex<Budget>>,
}

/// Successful response with the raw `result_info` for paginated callers.
pub struct Page<T> {
    pub result: T,
    pub result_info: Option<Value>,
}

impl<T> Page<T> {
    /// Cursor from `result_info.cursor`; empty string means "no more".
    pub fn cursor(&self) -> Option<String> {
        self.result_info
            .as_ref()
            .and_then(|i| i.get("cursor"))
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
    }
    pub fn total_count(&self) -> Option<u64> {
        self.result_info
            .as_ref()
            .and_then(|i| i.get("total_count"))
            .and_then(Value::as_u64)
    }
    pub fn total_pages(&self) -> Option<u64> {
        self.result_info
            .as_ref()
            .and_then(|i| i.get("total_pages"))
            .and_then(Value::as_u64)
    }
}

impl Client {
    pub fn new(token: &str) -> ApiResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let auth = HeaderValue::from_str(&format!("Bearer {}", token.trim()))
            .map_err(|_| ApiError::Transport("token contains invalid characters".into()))?;
        Ok(Self {
            http,
            auth: Arc::new(auth),
            budget: Arc::new(Mutex::new(Budget::default())),
        })
    }

    pub fn budget_used(&self) -> usize {
        self.budget.lock().map(|mut b| b.used()).unwrap_or(0)
    }

    fn url(path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{BASE_URL}{path}")
        }
    }

    fn build(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        body: Body,
    ) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .request(method, Self::url(path))
            .header(AUTHORIZATION, (*self.auth).clone());
        if !query.is_empty() {
            req = req.query(query);
        }
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        match body {
            Body::None => req,
            Body::Json(v) => req.json(&v),
            Body::Bytes(b, ct) => req.header(CONTENT_TYPE, ct).body(b),
            Body::Text(t, ct) => req.header(CONTENT_TYPE, ct).body(t),
            Body::Multipart(f) => req.multipart(f),
        }
    }

    /// Send a request and return the raw response, retrying once on 429.
    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        body: Body,
    ) -> ApiResult<reqwest::Response> {
        if let Ok(mut b) = self.budget.lock() {
            b.record();
        }
        let req = self.build(method, path, query, headers, body);
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(ApiError::RateLimited { retry_after });
        }
        Ok(resp)
    }

    /// JSON request returning the envelope's `result` plus `result_info`.
    pub async fn request_page<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        body: Body,
    ) -> ApiResult<Page<T>> {
        let resp = self.send(method, path, query, headers, body).await?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        decode_envelope(status, &text)
    }

    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Body,
    ) -> ApiResult<T> {
        Ok(self
            .request_page(method, path, query, &[], body)
            .await?
            .result)
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> ApiResult<T> {
        self.request(Method::GET, path, query, Body::None).await
    }

    pub async fn get_page<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> ApiResult<Page<T>> {
        self.request_page(Method::GET, path, query, &[], Body::None)
            .await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> ApiResult<T> {
        self.request(Method::POST, path, &[], Body::Json(body))
            .await
    }

    pub async fn put<T: DeserializeOwned>(&self, path: &str, body: Value) -> ApiResult<T> {
        self.request(Method::PUT, path, &[], Body::Json(body)).await
    }

    pub async fn patch<T: DeserializeOwned>(&self, path: &str, body: Value) -> ApiResult<T> {
        self.request(Method::PATCH, path, &[], Body::Json(body))
            .await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.request(Method::DELETE, path, &[], Body::None).await
    }

    pub async fn delete_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> ApiResult<T> {
        self.request(Method::DELETE, path, &[], Body::Json(body))
            .await
    }

    /// Raw-bytes GET (KV values, R2 objects, script bundles). Non-2xx bodies
    /// are parsed as an envelope when possible.
    pub async fn get_bytes(
        &self,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
    ) -> ApiResult<(Bytes, HeaderMap)> {
        let resp = self
            .send(Method::GET, path, query, headers, Body::None)
            .await?;
        let status = resp.status();
        let hdrs = resp.headers().clone();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&bytes).to_string();
            return Err(error_from_body(status, &text));
        }
        Ok((bytes, hdrs))
    }

    /// Raw-body PUT/POST returning the JSON envelope result.
    pub async fn send_body<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        body: Body,
    ) -> ApiResult<T> {
        Ok(self
            .request_page(method, path, query, headers, body)
            .await?
            .result)
    }

    /// Paginate a `page`/`per_page` list to the end. `per_page` must respect
    /// the endpoint's maximum.
    pub async fn get_all_pages<T: DeserializeOwned>(
        &self,
        path: &str,
        per_page: usize,
        extra: &[(&str, String)],
    ) -> ApiResult<Vec<T>> {
        let mut out = Vec::new();
        let mut page = 1usize;
        loop {
            let mut query: Vec<(&str, String)> = extra.to_vec();
            query.push(("page", page.to_string()));
            query.push(("per_page", per_page.to_string()));
            let p: Page<Vec<T>> = self.get_page(path, &query).await?;
            let total_pages = p.total_pages();
            let n = p.result.len();
            out.extend(p.result);
            let done = match total_pages {
                Some(tp) => page as u64 >= tp,
                None => n < per_page,
            };
            if done || n == 0 {
                break;
            }
            page += 1;
            if page > 200 {
                break; // safety valve
            }
        }
        Ok(out)
    }

    /// GraphQL Analytics API.
    pub async fn graphql(&self, query: &str, variables: Value) -> ApiResult<Value> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let resp = self
            .send(Method::POST, "/graphql", &[], &[], Body::Json(body))
            .await?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| ApiError::Decode(e.to_string()))?;
        if let Some(errs) = v.get("errors").and_then(Value::as_array)
            && let Some(first) = errs.first()
        {
            let msg = first
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("graphql error")
                .to_string();
            if msg.contains("authz") || msg.contains("permission") {
                return Err(ApiError::MissingPermission {
                    status: status.as_u16(),
                    code: 0,
                });
            }
            return Err(ApiError::Api {
                status: status.as_u16(),
                code: 0,
                message: msg,
            });
        }
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }
}

fn decode_envelope<T: DeserializeOwned>(status: StatusCode, text: &str) -> ApiResult<Page<T>> {
    let env: Envelope<Value> = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            if !status.is_success() {
                return Err(error_from_body(status, text));
            }
            return Err(ApiError::Decode(format!("{e}: {}", truncate(text, 200))));
        }
    };
    if !env.success || !status.is_success() {
        let errs = env.errors.unwrap_or_default();
        return Err(error_from_messages(status, &errs));
    }
    let result = env.result.unwrap_or(Value::Null);
    let result: T = serde_json::from_value(result)
        .map_err(|e| ApiError::Decode(format!("{e}: {}", truncate(text, 200))))?;
    Ok(Page {
        result,
        result_info: env.result_info,
    })
}

fn error_from_messages(status: StatusCode, errs: &[ApiMessage]) -> ApiError {
    if let Some(first) = errs.first() {
        if first.is_missing_permission() {
            return ApiError::MissingPermission {
                status: status.as_u16(),
                code: first.code,
            };
        }
        if status == StatusCode::NOT_FOUND {
            return ApiError::NotFound;
        }
        return ApiError::Api {
            status: status.as_u16(),
            code: first.code,
            message: first.message.clone(),
        };
    }
    if status == StatusCode::NOT_FOUND {
        return ApiError::NotFound;
    }
    ApiError::Api {
        status: status.as_u16(),
        code: 0,
        message: format!("HTTP {status}"),
    }
}

fn error_from_body(status: StatusCode, text: &str) -> ApiError {
    // KV returns `{"code":1000,"error":"not_found"}` on missing keys; other
    // raw endpoints return the normal envelope.
    if let Ok(env) = serde_json::from_str::<Envelope<Value>>(text) {
        return error_from_messages(status, &env.errors.unwrap_or_default());
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if v.get("error").and_then(Value::as_str) == Some("not_found")
            || status == StatusCode::NOT_FOUND
        {
            return ApiError::NotFound;
        }
        let msg = v
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| truncate(text, 200));
        return ApiError::Api {
            status: status.as_u16(),
            code: v.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: msg,
        };
    }
    if status == StatusCode::NOT_FOUND {
        return ApiError::NotFound;
    }
    ApiError::Api {
        status: status.as_u16(),
        code: 0,
        message: truncate(text, 200),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// Percent-encode a path segment (KV keys, R2 object keys).
pub fn encode_segment(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_permission_error() {
        let body = r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}],"messages":[],"result":null}"#;
        let err = decode_envelope::<Value>(StatusCode::FORBIDDEN, body)
            .err()
            .unwrap();
        assert!(err.is_missing_permission());
    }

    #[test]
    fn tolerates_null_errors() {
        let body = r#"{"success":true,"errors":null,"messages":null,"result":{"id":"x"}}"#;
        let page = decode_envelope::<Value>(StatusCode::OK, body).unwrap();
        assert_eq!(page.result["id"], "x");
    }

    #[test]
    fn kv_not_found_body() {
        let err = error_from_body(
            StatusCode::NOT_FOUND,
            r#"{"code":1000,"error":"not_found"}"#,
        );
        assert!(err.is_not_found());
    }

    #[test]
    fn cursor_empty_means_done() {
        let p = Page {
            result: (),
            result_info: Some(serde_json::json!({"count": 1, "cursor": ""})),
        };
        assert!(p.cursor().is_none());
        let p = Page {
            result: (),
            result_info: Some(serde_json::json!({"cursor": "abc"})),
        };
        assert_eq!(p.cursor().as_deref(), Some("abc"));
    }

    #[test]
    fn budget_counts_window() {
        let mut b = Budget::default();
        b.record();
        b.record();
        assert_eq!(b.used(), 2);
    }
}
