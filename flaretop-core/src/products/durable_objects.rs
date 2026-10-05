//! Durable Objects: inventory only. Namespaces and object ids are listable;
//! storage, alarms and deletion are not exposed by the API.

use serde::Deserialize;

use crate::api::{ApiResult, Client, Page};

#[derive(Debug, Clone, Deserialize)]
pub struct Namespace {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub use_sqlite: Option<bool>,
}

pub async fn namespaces(c: &Client, account: &str) -> ApiResult<Vec<Namespace>> {
    let mut v: Vec<Namespace> = c
        .get_all_pages(
            &format!("/accounts/{account}/workers/durable_objects/namespaces"),
            1000,
            &[],
        )
        .await?;
    v.sort_by(|a, b| a.script.cmp(&b.script).then(a.class.cmp(&b.class)));
    Ok(v)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObjectId {
    pub id: String,
    #[serde(rename = "hasStoredData", default)]
    pub has_stored_data: bool,
}

pub struct ObjectPage {
    pub objects: Vec<ObjectId>,
    pub cursor: Option<String>,
}

/// The server rejects `limit` below 100 in practice.
pub async fn objects(
    c: &Client,
    account: &str,
    ns: &str,
    cursor: Option<&str>,
) -> ApiResult<ObjectPage> {
    let mut q: Vec<(&str, String)> = vec![("limit", "100".into())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur.to_string()));
    }
    let p: Page<Vec<ObjectId>> = c
        .get_page(
            &format!("/accounts/{account}/workers/durable_objects/namespaces/{ns}/objects"),
            &q,
        )
        .await?;
    Ok(ObjectPage {
        cursor: p.cursor(),
        objects: p.result,
    })
}
