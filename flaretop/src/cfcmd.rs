//! Best-effort `cf` CLI equivalents for what the UI is looking at, so an
//! action can be copied into a script or handed to an agent.

use crate::app::{App, Section};

fn sh(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:@=".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn for_context(app: &App) -> Option<String> {
    Some(match app.section {
        Section::Workers => {
            let s = app.workers.list.current()?;
            match app.focus {
                crate::app::Focus::Detail if app.workers.versions_mode => {
                    let d = app.workers.detail.as_ref()?;
                    let v = d.versions.get(app.workers.versions.sel)?;
                    format!(
                        "cf workers scripts deployments create {} --versions '[{{\"version_id\":\"{}\",\"percentage\":100}}]'",
                        sh(&s.id),
                        v.id
                    )
                }
                _ => format!("cf workers scripts get {}", sh(&s.id)),
            }
        }
        Section::Pages => {
            let p = app.pages.list.current()?;
            if app.focus == crate::app::Focus::Detail
                && let Some(d) = app.pages.deployments.get(app.pages.dep.sel)
            {
                return Some(format!(
                    "cf pages projects deployments get {} {}",
                    sh(&p.name),
                    sh(&d.id)
                ));
            }
            format!("cf pages projects get {}", sh(&p.name))
        }
        Section::D1 => {
            let db = app.d1.list.current()?;
            if app.focus == crate::app::Focus::Detail && !app.d1.last_sql.is_empty() {
                format!("cf d1 raw {} --sql {}", sh(&db.uuid), sh(&app.d1.last_sql))
            } else {
                format!("cf d1 get {}", sh(&db.uuid))
            }
        }
        Section::Kv => {
            let ns = app.kv.list.current()?;
            if app.focus == crate::app::Focus::Detail
                && let Some(k) = app.kv.keys.get(app.kv.key.sel)
            {
                return Some(format!(
                    "cf kv values get --namespace-id {} --key-name {}",
                    sh(&ns.id),
                    sh(&k.name)
                ));
            }
            format!("cf kv keys list --namespace-id {}", sh(&ns.id))
        }
        Section::R2 => {
            let b = app.r2.list.current()?;
            if app.focus == crate::app::Focus::Detail
                && let Some(crate::app::R2Entry::Object(o)) = app.r2.entries.get(app.r2.entry.sel)
            {
                return Some(format!(
                    "cf r2 buckets objects get {} --key {}",
                    sh(&b.name),
                    sh(&o.key)
                ));
            }
            format!(
                "cf r2 buckets objects list {} --prefix {}",
                sh(&b.name),
                sh(&app.r2.prefix)
            )
        }
        Section::Queues => {
            let q = app.queues.list.current()?;
            format!("cf queues get {}", sh(&q.queue_id))
        }
        Section::DurableObjects => {
            let n = app.dos.list.current()?;
            format!(
                "cf workers durable-objects namespaces objects list {}",
                sh(&n.id)
            )
        }
    })
}
