//! The I/O actor: owns the API client and runs every command off the render
//! path. Commands run concurrently (bounded) and each answer carries the
//! UI's `req_id` so stale results can be dropped.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::api::{ApiError, ApiResult, Client};
use crate::auth;
use crate::products::{d1, durable_objects as dos, kv, pages, queues, r2, workers};
use crate::tail::{self, TailFilters, TailMsg};
use crate::types::{BucketInfo, Command, CoreEvent, Payload, ScriptDetail};

const CONCURRENCY: usize = 6;

pub struct Handle {
    pub tx: mpsc::Sender<(u64, Command)>,
    pub rx: mpsc::Receiver<CoreEvent>,
}

pub fn spawn(client: Client, account: String, read_only: bool) -> Handle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<(u64, Command)>(64);
    let (evt_tx, evt_rx) = mpsc::channel::<CoreEvent>(256);
    tokio::spawn(run(client, account, read_only, cmd_rx, evt_tx));
    Handle {
        tx: cmd_tx,
        rx: evt_rx,
    }
}

struct TailTask {
    stop: oneshot::Sender<()>,
    script: String,
    tail_id: String,
}

async fn run(
    client: Client,
    account: String,
    read_only: bool,
    mut cmds: mpsc::Receiver<(u64, Command)>,
    events: mpsc::Sender<CoreEvent>,
) {
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let mut account = account;
    let mut tail_task: Option<TailTask> = None;
    while let Some((req_id, cmd)) = cmds.recv().await {
        match cmd {
            Command::SetAccount(id) => {
                account = id;
                continue;
            }
            Command::StartTail { script, filters } => {
                stop_tail(&client, &account, tail_task.take()).await;
                match start_tail(&client, &account, &script, &filters, events.clone()).await {
                    Ok(t) => tail_task = Some(t),
                    Err(e) => {
                        let _ = events.send(CoreEvent::TailClosed(e.to_string())).await;
                    }
                }
                continue;
            }
            Command::StopTail => {
                stop_tail(&client, &account, tail_task.take()).await;
                continue;
            }
            _ => {}
        }
        if read_only && is_write(&cmd) {
            let _ = events
                .send(CoreEvent::Done {
                    req_id,
                    result: Err(ApiError::Api {
                        status: 0,
                        code: 0,
                        message: "read-only mode: writes are blocked".into(),
                    }),
                })
                .await;
            continue;
        }
        let client = client.clone();
        let account = account.clone();
        let events = events.clone();
        let sem = sem.clone();
        tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            let result = handle(&client, &account, cmd).await;
            let _ = events.send(CoreEvent::Done { req_id, result }).await;
        });
    }
    stop_tail(&client, &account, tail_task.take()).await;
}

fn is_write(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::PutSecret { .. }
            | Command::DeleteSecret { .. }
            | Command::PutSchedules { .. }
            | Command::SetSubdomain { .. }
            | Command::Deploy { .. }
            | Command::DeleteScript { .. }
            | Command::PagesRollback { .. }
            | Command::PagesRetry { .. }
            | Command::DeletePagesProject { .. }
            | Command::CreateDatabase { .. }
            | Command::DeleteDatabase { .. }
            | Command::PutValue { .. }
            | Command::DeleteKey { .. }
            | Command::BulkDeleteKeys { .. }
            | Command::CreateNamespace { .. }
            | Command::DeleteNamespace { .. }
            | Command::PutObject { .. }
            | Command::DeleteObject { .. }
            | Command::DeleteObjects { .. }
            | Command::CreateBucket { .. }
            | Command::DeleteBucket { .. }
            | Command::SetManagedDomain { .. }
            | Command::PushMessage { .. }
            | Command::PurgeQueue { .. }
            | Command::SetQueuePaused { .. }
            | Command::CreateQueue { .. }
            | Command::DeleteQueue { .. }
            | Command::PullMessages { .. }
            | Command::AckMessages { .. }
    )
}

/// Statements that modify data; `RunSql` is checked separately because the
/// read-only gate must allow SELECT.
pub fn sql_is_write(sql: &str) -> bool {
    let head = sql
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    !matches!(head.as_str(), "SELECT" | "WITH" | "EXPLAIN" | "PRAGMA" | "")
}

async fn start_tail(
    client: &Client,
    account: &str,
    script: &str,
    filters: &TailFilters,
    events: mpsc::Sender<CoreEvent>,
) -> ApiResult<TailTask> {
    let t = tail::create(client, account, script, filters).await?;
    let (stop_tx, stop_rx) = oneshot::channel();
    let (msg_tx, mut msg_rx) = mpsc::channel::<TailMsg>(512);
    tokio::spawn(tail::run(t.url.clone(), msg_tx, stop_rx));
    tokio::spawn(async move {
        while let Some(m) = msg_rx.recv().await {
            let ev = match m {
                TailMsg::Event(e) => CoreEvent::Tail(e),
                TailMsg::Info(s) => CoreEvent::TailInfo(s),
                TailMsg::Closed(s) => CoreEvent::TailClosed(s),
            };
            if events.send(ev).await.is_err() {
                break;
            }
        }
    });
    Ok(TailTask {
        stop: stop_tx,
        script: script.to_string(),
        tail_id: t.id,
    })
}

async fn stop_tail(client: &Client, account: &str, task: Option<TailTask>) {
    if let Some(t) = task {
        let _ = t.stop.send(());
        let _ = tail::delete(client, account, &t.script, &t.tail_id).await;
    }
}

async fn handle(c: &Client, a: &str, cmd: Command) -> ApiResult<Payload> {
    use Command as C;
    Ok(match cmd {
        C::Verify => Payload::Verified(auth::verify(c).await?),
        C::ListAccounts => Payload::Accounts(auth::accounts(c).await?),
        C::SetAccount(_) | C::StartTail { .. } | C::StopTail => unreachable!("handled in run loop"),

        C::ListScripts => Payload::Scripts(workers::list(c, a).await?),
        C::LoadScript { name } => Payload::Script(load_script(c, a, &name).await),
        C::LoadScriptContent { name } => {
            let (body, ct) = workers::content(c, a, &name).await?;
            Payload::ScriptContent {
                name,
                parts: workers::split_multipart(&body, &ct),
            }
        }
        C::PutSecret { name, key, value } => {
            workers::put_secret(c, a, &name, &key, &value).await?;
            written(format!("set secret {key} on {name}"))
        }
        C::DeleteSecret { name, key } => {
            workers::delete_secret(c, a, &name, &key).await?;
            written(format!("deleted secret {key} on {name}"))
        }
        C::PutSchedules { name, crons } => {
            workers::put_schedules(c, a, &name, &crons).await?;
            written(format!(
                "cron triggers of {name} set to [{}]",
                crons.join(", ")
            ))
        }
        C::SetSubdomain {
            name,
            enabled,
            previews,
        } => {
            workers::set_subdomain(c, a, &name, enabled, previews).await?;
            written(format!(
                "workers.dev for {name} {}",
                if enabled { "enabled" } else { "disabled" }
            ))
        }
        C::Deploy {
            name,
            versions,
            message,
        } => {
            workers::create_deployment(c, a, &name, &versions, message.as_deref()).await?;
            let desc: Vec<String> = versions
                .iter()
                .map(|(id, p)| format!("{}@{p}%", &id[..id.len().min(8)]))
                .collect();
            written(format!("deployed {name}: {}", desc.join(" + ")))
        }
        C::DeleteScript { name } => {
            workers::delete_script(c, a, &name, true).await?;
            written(format!("deleted worker {name}"))
        }

        C::ListPagesProjects => Payload::PagesProjects(pages::list(c, a).await?),
        C::LoadPagesDeployments { project } => {
            let items = pages::deployments(c, a, &project).await?;
            Payload::PagesDeployments { project, items }
        }
        C::PagesRollback { project, id } => {
            pages::rollback(c, a, &project, &id).await?;
            written(format!(
                "rolled {project} back to deployment {}",
                &id[..8.min(id.len())]
            ))
        }
        C::PagesRetry { project, id } => {
            pages::retry(c, a, &project, &id).await?;
            written(format!(
                "retrying deployment {} of {project}",
                &id[..8.min(id.len())]
            ))
        }
        C::PagesLogs { project, id } => {
            let lines = pages::logs(c, a, &project, &id).await?;
            Payload::PagesLogs { project, id, lines }
        }
        C::DeletePagesProject { name } => {
            pages::delete_project(c, a, &name).await?;
            written(format!("deleted Pages project {name}"))
        }

        C::ListDatabases => Payload::Databases(d1::list(c, a).await?),
        C::GetDatabase { id } => Payload::Database(d1::get(c, a, &id).await?),
        C::LoadTables { db } => {
            let tables = d1::tables(c, a, &db).await?;
            Payload::Tables { db, tables }
        }
        C::LoadColumns { db, table } => {
            let columns = d1::columns(c, a, &db, &table).await?;
            Payload::Columns { db, table, columns }
        }
        C::RunSql { db, sql } => {
            let results = d1::raw(c, a, &db, &sql, &[]).await?;
            Payload::SqlResult { db, sql, results }
        }
        C::CreateDatabase { name } => {
            d1::create(c, a, &name, None).await?;
            written(format!("created D1 database {name}"))
        }
        C::DeleteDatabase { id, name } => {
            d1::delete(c, a, &id).await?;
            written(format!("deleted D1 database {name}"))
        }
        C::D1Bookmark { id } => {
            let bookmark = d1::bookmark(c, a, &id, None).await?;
            Payload::Bookmark { id, bookmark }
        }

        C::ListNamespaces => Payload::Namespaces(kv::namespaces(c, a).await?),
        C::ListKeys { ns, prefix, cursor } => {
            let append = cursor.is_some();
            let page = kv::keys(c, a, &ns, &prefix, cursor.as_deref(), 500).await?;
            Payload::Keys {
                ns,
                prefix,
                page: page.keys,
                cursor: page.cursor,
                append,
            }
        }
        C::GetValue { ns, key } => {
            let (bytes, expiration) = kv::get_value(c, a, &ns, &key).await?;
            let metadata = kv::metadata(c, a, &ns, &key)
                .await
                .ok()
                .filter(|m| !m.is_null());
            Payload::Value {
                ns,
                key,
                bytes: bytes.to_vec(),
                expiration,
                metadata,
            }
        }
        C::PutValue {
            ns,
            key,
            value,
            metadata,
            ttl,
        } => {
            kv::put_value(c, a, &ns, &key, value, metadata, ttl).await?;
            written(format!("put key {key}"))
        }
        C::DeleteKey { ns, key } => {
            kv::delete_key(c, a, &ns, &key).await?;
            written(format!("deleted key {key}"))
        }
        C::BulkDeleteKeys { ns, keys } => {
            let n = keys.len();
            for chunk in keys.chunks(10_000) {
                kv::bulk_delete(c, a, &ns, chunk).await?;
            }
            written(format!("deleted {n} keys"))
        }
        C::CreateNamespace { title } => {
            kv::create_namespace(c, a, &title).await?;
            written(format!("created KV namespace {title}"))
        }
        C::DeleteNamespace { id, title } => {
            kv::delete_namespace(c, a, &id).await?;
            written(format!("deleted KV namespace {title}"))
        }

        C::ListBuckets => Payload::Buckets(r2::buckets(c, a).await?),
        C::ListObjects {
            bucket,
            prefix,
            cursor,
        } => {
            let append = cursor.is_some();
            let page =
                r2::objects(c, a, &bucket, &prefix, Some("/"), cursor.as_deref(), 500).await?;
            Payload::Objects {
                bucket,
                prefix,
                objects: page.objects,
                folders: page.folders,
                cursor: page.cursor,
                append,
            }
        }
        C::GetObject { bucket, key } => {
            let (bytes, content_type) = r2::get_object(c, a, &bucket, &key).await?;
            Payload::Object {
                bucket,
                key,
                bytes: bytes.to_vec(),
                content_type,
            }
        }
        C::PutObject {
            bucket,
            key,
            bytes,
            content_type,
        } => {
            let n = bytes.len() as u64;
            r2::put_object(c, a, &bucket, &key, bytes, &content_type).await?;
            written(format!(
                "uploaded {key} ({}) to {bucket}",
                crate::human_bytes(n)
            ))
        }
        C::DeleteObject { bucket, key } => {
            r2::delete_object(c, a, &bucket, &key).await?;
            written(format!("deleted {key} from {bucket}"))
        }
        C::DeleteObjects { bucket, keys } => {
            let n = keys.len();
            for chunk in keys.chunks(1000) {
                r2::delete_objects(c, a, &bucket, chunk).await?;
            }
            written(format!("deleted {n} objects from {bucket}"))
        }
        C::CreateBucket { name } => {
            r2::create_bucket(c, a, &name, None).await?;
            written(format!("created bucket {name}"))
        }
        C::DeleteBucket { name } => {
            r2::delete_bucket(c, a, &name).await?;
            written(format!("deleted bucket {name}"))
        }
        C::LoadBucketInfo { name } => {
            let bucket = r2::get_bucket(c, a, &name).await?;
            let (managed, custom, stats) = tokio::join!(
                r2::managed_domain(c, a, &name),
                r2::custom_domains(c, a, &name),
                r2::bucket_stats(c, a, &name)
            );
            Payload::BucketInfo(BucketInfo {
                bucket,
                managed: managed.ok(),
                custom: custom.unwrap_or_default(),
                stats: stats.ok(),
            })
        }
        C::SetManagedDomain { bucket, enabled } => {
            r2::set_managed_domain(c, a, &bucket, enabled).await?;
            written(format!(
                "r2.dev public access for {bucket} {}",
                if enabled { "enabled" } else { "disabled" }
            ))
        }

        C::ListQueues => Payload::Queues(queues::list(c, a).await?),
        C::PushMessage { id, body, json } => {
            queues::push(c, a, &id, &body, json, None).await?;
            written("pushed 1 message".into())
        }
        C::PurgeQueue { id, name } => {
            queues::purge(c, a, &id).await?;
            written(format!("purged queue {name}"))
        }
        C::SetQueuePaused { id, paused } => {
            queues::set_paused(c, a, &id, paused).await?;
            written(format!(
                "queue delivery {}",
                if paused { "paused" } else { "resumed" }
            ))
        }
        C::CreateQueue { name } => {
            queues::create(c, a, &name).await?;
            written(format!("created queue {name}"))
        }
        C::DeleteQueue { id, name } => {
            queues::delete(c, a, &id).await?;
            written(format!("deleted queue {name}"))
        }
        C::PullMessages { id, batch } => {
            let result = queues::pull(c, a, &id, batch, 30_000).await?;
            Payload::Pulled { id, result }
        }
        C::AckMessages { id, acks, retries } => {
            let (na, nr) = (acks.len(), retries.len());
            queues::ack(c, a, &id, &acks, &retries).await?;
            written(format!("acked {na}, retried {nr}"))
        }

        C::ListDoNamespaces => Payload::DoNamespaces(dos::namespaces(c, a).await?),
        C::ListDoObjects { ns, cursor } => {
            let append = cursor.is_some();
            let page = dos::objects(c, a, &ns, cursor.as_deref()).await?;
            Payload::DoObjects {
                ns,
                objects: page.objects,
                cursor: page.cursor,
                append,
            }
        }
    })
}

fn written(what: String) -> Payload {
    Payload::Written { what }
}

async fn load_script(c: &Client, a: &str, name: &str) -> ScriptDetail {
    let (settings, schedules, secrets, subdomain, versions, deployments, domains, stats, acct_sub) = tokio::join!(
        workers::settings(c, a, name),
        workers::schedules(c, a, name),
        workers::secrets(c, a, name),
        workers::subdomain(c, a, name),
        workers::versions(c, a, name),
        workers::deployments(c, a, name),
        workers::domains(c, a),
        workers::invocation_stats_24h(c, a, name),
        workers::account_subdomain(c, a),
    );
    let mut errors = Vec::new();
    let mut note = |label: &str, e: &ApiError| {
        errors.push(format!("{label}: {e}"));
    };
    let settings = settings.map_err(|e| note("settings", &e)).ok();
    let schedules = schedules.map_err(|e| note("cron", &e)).unwrap_or_default();
    let secrets = secrets.map_err(|e| note("secrets", &e)).unwrap_or_default();
    let subdomain = subdomain.map_err(|e| note("subdomain", &e)).ok();
    let versions = versions
        .map_err(|e| note("versions", &e))
        .unwrap_or_default();
    let deployments = deployments
        .map_err(|e| note("deployments", &e))
        .unwrap_or_default();
    let domains = domains
        .map(|d| d.into_iter().filter(|d| d.service == name).collect())
        .map_err(|e| note("domains", &e))
        .unwrap_or_default();
    let stats = stats.ok();
    let account_subdomain = acct_sub.ok();
    ScriptDetail {
        name: name.to_string(),
        settings,
        schedules,
        secrets,
        subdomain,
        versions,
        deployments,
        domains,
        stats,
        account_subdomain,
        errors,
    }
}

/// Bindings from a settings object as (type, name, target) rows.
pub fn bindings_rows(settings: &Value) -> Vec<(String, String, String)> {
    let Some(arr) = settings.get("bindings").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .map(|b| {
            let ty = b
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let name = b
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let target = match ty.as_str() {
                "d1" => b.get("id").or(b.get("database_id")),
                "kv_namespace" => b.get("namespace_id"),
                "r2_bucket" => b.get("bucket_name"),
                "queue" => b.get("queue_name"),
                "durable_object_namespace" => b.get("class_name"),
                "service" => b.get("service"),
                "plain_text" => b.get("text"),
                "json" => b.get("json"),
                "vectorize" => b.get("index_name"),
                "hyperdrive" => b.get("id"),
                "workflow" => b.get("workflow_name"),
                "analytics_engine" => b.get("dataset"),
                "dispatch_namespace" => b.get("namespace"),
                _ => None,
            }
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
            (ty, name, target)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_write_detection() {
        assert!(!sql_is_write("select 1"));
        assert!(!sql_is_write("  WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(sql_is_write("DELETE FROM t"));
        assert!(sql_is_write("insert into t values (1)"));
    }

    #[test]
    fn bindings_table() {
        let s = serde_json::json!({ "bindings": [
            { "type": "d1", "name": "DB", "id": "abc" },
            { "type": "secret_text", "name": "KEY" }
        ]});
        let rows = bindings_rows(&s);
        assert_eq!(rows[0], ("d1".into(), "DB".into(), "abc".into()));
        assert_eq!(rows[1].2, "");
    }
}
