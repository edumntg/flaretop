//! Cloudflare Pages: projects, deployments, rollback, retry, build logs.
//! The product is in maintenance (Workers static assets is the successor)
//! but the API is complete and stable.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Client};

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub subdomain: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub production_branch: Option<String>,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub source: Option<Value>,
    #[serde(default)]
    pub build_config: Option<Value>,
    #[serde(default)]
    pub latest_deployment: Option<Value>,
    #[serde(default)]
    pub canonical_deployment: Option<Value>,
    #[serde(default)]
    pub framework: Option<String>,
    #[serde(default)]
    pub production_script_name: Option<String>,
}

impl Project {
    /// "github owner/repo" when the project builds from Git.
    pub fn repo(&self) -> Option<String> {
        let src = self.source.as_ref()?;
        let ty = src.get("type")?.as_str()?;
        let cfg = src.get("config")?;
        Some(format!(
            "{ty} {}/{}",
            cfg.get("owner")?.as_str()?,
            cfg.get("repo_name")?.as_str()?
        ))
    }
    pub fn latest_status(&self) -> Option<String> {
        let d = self.latest_deployment.as_ref()?;
        let stage = d.get("latest_stage")?;
        Some(format!(
            "{} {}",
            stage.get("name")?.as_str()?,
            stage.get("status")?.as_str()?
        ))
    }
    pub fn latest_modified(&self) -> Option<&str> {
        self.latest_deployment
            .as_ref()?
            .get("modified_on")?
            .as_str()
    }
}

pub async fn list(c: &Client, account: &str) -> ApiResult<Vec<Project>> {
    let mut v: Vec<Project> = c
        .get_all_pages(&format!("/accounts/{account}/pages/projects"), 100, &[])
        .await?;
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

pub async fn get(c: &Client, account: &str, name: &str) -> ApiResult<Project> {
    c.get(&format!("/accounts/{account}/pages/projects/{name}"), &[])
        .await
}

pub async fn delete_project(c: &Client, account: &str, name: &str) -> ApiResult<Value> {
    c.delete(&format!("/accounts/{account}/pages/projects/{name}"))
        .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Stage {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    pub id: String,
    #[serde(default)]
    pub short_id: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
    #[serde(default)]
    pub created_on: Option<String>,
    #[serde(default)]
    pub modified_on: Option<String>,
    #[serde(default)]
    pub latest_stage: Option<Stage>,
    #[serde(default)]
    pub deployment_trigger: Option<Value>,
    #[serde(default)]
    pub is_skipped: Option<bool>,
}

impl Deployment {
    pub fn status(&self) -> String {
        match &self.latest_stage {
            Some(s) => format!(
                "{}:{}",
                s.name.clone().unwrap_or_default(),
                s.status.clone().unwrap_or_default()
            ),
            None => String::new(),
        }
    }
    pub fn branch(&self) -> Option<&str> {
        self.deployment_trigger
            .as_ref()?
            .pointer("/metadata/branch")?
            .as_str()
    }
    pub fn commit_message(&self) -> Option<&str> {
        self.deployment_trigger
            .as_ref()?
            .pointer("/metadata/commit_message")?
            .as_str()
    }
    pub fn commit_hash(&self) -> Option<&str> {
        self.deployment_trigger
            .as_ref()?
            .pointer("/metadata/commit_hash")?
            .as_str()
    }
    pub fn trigger(&self) -> Option<&str> {
        self.deployment_trigger.as_ref()?.get("type")?.as_str()
    }
    pub fn succeeded(&self) -> bool {
        self.latest_stage
            .as_ref()
            .map(|s| s.status.as_deref() == Some("success"))
            .unwrap_or(false)
    }
}

/// Newest first; both environments.
pub async fn deployments(c: &Client, account: &str, project: &str) -> ApiResult<Vec<Deployment>> {
    c.get(
        &format!("/accounts/{account}/pages/projects/{project}/deployments"),
        &[("per_page", "25".into())],
    )
    .await
}

/// Makes this deployment the production one.
pub async fn rollback(c: &Client, account: &str, project: &str, id: &str) -> ApiResult<Value> {
    c.post(
        &format!("/accounts/{account}/pages/projects/{project}/deployments/{id}/rollback"),
        json!({}),
    )
    .await
}

/// Re-runs the build of a failed or cancelled deployment.
pub async fn retry(c: &Client, account: &str, project: &str, id: &str) -> ApiResult<Value> {
    c.post(
        &format!("/accounts/{account}/pages/projects/{project}/deployments/{id}/retry"),
        json!({}),
    )
    .await
}

#[derive(Debug, Deserialize)]
struct LogLine {
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    line: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Logs {
    #[serde(default)]
    data: Vec<LogLine>,
}

pub async fn logs(c: &Client, account: &str, project: &str, id: &str) -> ApiResult<Vec<String>> {
    let l: Logs = c
        .get(
            &format!("/accounts/{account}/pages/projects/{project}/deployments/{id}/history/logs"),
            &[],
        )
        .await?;
    Ok(l.data
        .into_iter()
        .map(|x| {
            format!(
                "{} {}",
                x.ts.as_deref().map(crate::short_time).unwrap_or_default(),
                x.line.unwrap_or_default()
            )
        })
        .collect())
}
