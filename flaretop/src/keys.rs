//! Keyboard and mouse handling.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use flaretop_core::products::d1;
use flaretop_core::tail::TailFilters;
use flaretop_core::types::Command;

use crate::actions::{empty_row_json, row_to_json};
use crate::app::{App, D1View, Focus, R2Entry, SECTIONS, Section, TailState};
use crate::modal::{EditorAction, Modal, PendingAction, PromptAction};
use crate::util;

pub fn on_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    if app.modal.is_open() {
        on_key_modal(app, key);
        return;
    }
    if app.tail.is_some() {
        on_key_tail(app, key);
        return;
    }
    if app.filter_mode {
        on_key_filter(app, key);
        return;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('c') if ctrl => app.should_quit = true,
        KeyCode::Char('?') => app.open_help(),
        KeyCode::Char(':') => app.open_palette(),
        KeyCode::Char('p') if ctrl => app.open_palette(),
        KeyCode::Tab => cycle_focus(app, false),
        KeyCode::BackTab => cycle_focus(app, true),
        KeyCode::Char(c @ '1'..='7') => {
            let i = (c as u8 - b'1') as usize;
            if i < SECTIONS.len() {
                app.section = SECTIONS[i];
                app.focus = Focus::List;
                app.ensure_loaded();
                app.load_selection();
            }
        }
        KeyCode::Char('r') if !ctrl => app.refresh(),
        KeyCode::Char('/') => {
            app.filter_mode = true;
            app.focus = Focus::List;
        }
        KeyCode::Char('L') => app.modal = Modal::OpsLog { scroll: usize::MAX },
        KeyCode::Char('A') => app.run_action(crate::modal::AppAction::Accounts),
        KeyCode::Char('P') => app.run_action(crate::modal::AppAction::Profiles),
        KeyCode::Char('C') => app.copy_cf(),
        KeyCode::Char('[') => app.resize_list(-3),
        KeyCode::Char(']') => app.resize_list(3),
        KeyCode::Char('{') => app.resize_sidebar(-2),
        KeyCode::Char('}') => app.resize_sidebar(2),
        KeyCode::Char('=') => app.reset_sizes(),
        KeyCode::Esc => {
            if app.section == Section::D1
                && app.d1.view == D1View::Grid
                && app.focus == Focus::Detail
            {
                app.d1_back_to_tables();
            } else if app.focus != Focus::List {
                app.focus = Focus::List;
            }
        }
        _ => match app.focus {
            Focus::Sidebar => on_key_sidebar(app, key),
            Focus::List => on_key_list(app, key),
            Focus::Detail => on_key_detail(app, key),
        },
    }
}

fn cycle_focus(app: &mut App, back: bool) {
    app.focus = match (app.focus, back) {
        (Focus::Sidebar, false) => Focus::List,
        (Focus::List, false) => Focus::Detail,
        (Focus::Detail, false) => Focus::Sidebar,
        (Focus::Sidebar, true) => Focus::Detail,
        (Focus::List, true) => Focus::Sidebar,
        (Focus::Detail, true) => Focus::List,
    };
    if app.focus == Focus::Detail {
        app.load_selection();
    }
}

fn on_key_filter(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.filter_mode = false;
            set_filter(app, String::new());
        }
        KeyCode::Enter => {
            app.filter_mode = false;
            app.load_selection();
        }
        KeyCode::Backspace => {
            let mut f = current_filter(app);
            f.pop();
            set_filter(app, f);
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut f = current_filter(app);
            f.push(c);
            set_filter(app, f);
        }
        KeyCode::Down => move_list(app, 1),
        KeyCode::Up => move_list(app, -1),
        _ => {}
    }
}

pub fn current_filter(app: &App) -> String {
    match app.section {
        Section::Workers => app.workers.list.filter.clone(),
        Section::Pages => app.pages.list.filter.clone(),
        Section::D1 => app.d1.list.filter.clone(),
        Section::Kv => app.kv.list.filter.clone(),
        Section::R2 => app.r2.list.filter.clone(),
        Section::Queues => app.queues.list.filter.clone(),
        Section::DurableObjects => app.dos.list.filter.clone(),
    }
}

fn set_filter(app: &mut App, f: String) {
    match app.section {
        Section::Workers => {
            app.workers.list.filter = f;
            app.workers.list.refilter(|s| s.id.clone());
        }
        Section::Pages => {
            app.pages.list.filter = f;
            app.pages.list.refilter(|p| p.name.clone());
        }
        Section::D1 => {
            app.d1.list.filter = f;
            app.d1.list.refilter(|d| d.name.clone());
        }
        Section::Kv => {
            app.kv.list.filter = f;
            app.kv.list.refilter(|n| n.title.clone());
        }
        Section::R2 => {
            app.r2.list.filter = f;
            app.r2.list.refilter(|b| b.name.clone());
        }
        Section::Queues => {
            app.queues.list.filter = f;
            app.queues.list.refilter(|q| q.queue_name.clone());
        }
        Section::DurableObjects => {
            app.dos.list.filter = f;
            app.dos.list.refilter(|n| {
                format!(
                    "{} {}",
                    n.script.clone().unwrap_or_default(),
                    n.class.clone().unwrap_or_default()
                )
            });
        }
    }
}

fn on_key_sidebar(app: &mut App, key: KeyEvent) {
    let i = app.section.index();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.section = SECTIONS[(i + 1).min(SECTIONS.len() - 1)];
            app.ensure_loaded();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.section = SECTIONS[i.saturating_sub(1)];
            app.ensure_loaded();
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            app.focus = Focus::List;
            app.load_selection();
        }
        _ => {}
    }
}

fn move_list(app: &mut App, delta: i64) {
    match app.section {
        Section::Workers => app.workers.list.move_by(delta),
        Section::Pages => app.pages.list.move_by(delta),
        Section::D1 => app.d1.list.move_by(delta),
        Section::Kv => app.kv.list.move_by(delta),
        Section::R2 => app.r2.list.move_by(delta),
        Section::Queues => app.queues.list.move_by(delta),
        Section::DurableObjects => app.dos.list.move_by(delta),
    }
    app.select_changed = Some(std::time::Instant::now());
}

pub fn list_len(app: &App) -> usize {
    match app.section {
        Section::Workers => app.workers.list.len(),
        Section::Pages => app.pages.list.len(),
        Section::D1 => app.d1.list.len(),
        Section::Kv => app.kv.list.len(),
        Section::R2 => app.r2.list.len(),
        Section::Queues => app.queues.list.len(),
        Section::DurableObjects => app.dos.list.len(),
    }
}

fn on_key_list(app: &mut App, key: KeyEvent) {
    let page = app.rects.list_inner.height.max(1) as i64;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_list(app, 1),
        KeyCode::Char('k') | KeyCode::Up => move_list(app, -1),
        KeyCode::PageDown => move_list(app, page),
        KeyCode::PageUp => move_list(app, -page),
        KeyCode::Char('g') | KeyCode::Home => move_list(app, -(i64::MAX / 2)),
        KeyCode::Char('G') | KeyCode::End => move_list(app, i64::MAX / 2),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            app.select_changed = None;
            app.load_selection();
            app.focus = Focus::Detail;
        }
        KeyCode::Char('h') | KeyCode::Left => app.focus = Focus::Sidebar,
        KeyCode::Char(c) => section_key(app, c),
        _ => {}
    }
}

fn on_key_detail(app: &mut App, key: KeyEvent) {
    let page = app.rects.detail_inner.height.saturating_sub(2).max(1) as i64;
    let delta = match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(1),
        KeyCode::Char('k') | KeyCode::Up => Some(-1),
        KeyCode::PageDown => Some(page),
        KeyCode::PageUp => Some(-page),
        KeyCode::Char('g') | KeyCode::Home => Some(-(i64::MAX / 2)),
        KeyCode::End => Some(i64::MAX / 2),
        _ => None,
    };
    if let Some(d) = delta {
        move_detail(app, d);
        return;
    }
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => {
            if app.section == Section::D1 && app.d1.view == D1View::Grid {
                app.d1.hscroll = app.d1.hscroll.saturating_sub(1);
            } else {
                app.focus = Focus::List;
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if app.section == Section::D1 && app.d1.view == D1View::Grid {
                let ncols = app.d1.grid.as_ref().map(|g| g.columns.len()).unwrap_or(0);
                app.d1.hscroll = (app.d1.hscroll + 1).min(ncols.saturating_sub(1));
            }
        }
        KeyCode::Enter => detail_enter(app),
        KeyCode::Backspace => {
            if app.section == Section::R2 {
                r2_up(app);
            }
        }
        KeyCode::Char(c) => section_key(app, c),
        _ => {}
    }
}

fn move_detail(app: &mut App, delta: i64) {
    match app.section {
        Section::Workers => {
            if app.workers.versions_mode {
                let n = app
                    .workers
                    .detail
                    .as_ref()
                    .map(|d| d.versions.len())
                    .unwrap_or(0);
                app.workers.versions.move_by(delta, n);
            } else {
                app.workers.scroll = (app.workers.scroll as i64 + delta).max(0) as usize;
            }
        }
        Section::Pages => app.pages.dep.move_by(delta, app.pages.deployments.len()),
        Section::D1 => match app.d1.view {
            D1View::Tables => app.d1.table.move_by(delta, app.d1.tables.len()),
            D1View::Grid => {
                let n = app.d1.grid.as_ref().map(|g| g.rows.len()).unwrap_or(0);
                app.d1.row.move_by(delta, n);
            }
        },
        Section::Kv => {
            let n = app.kv.keys.len();
            app.kv.key.move_by(delta, n);
            if n > 0 && app.kv.key.sel + 1 >= n && app.kv.cursor_token.is_some() && !app.kv.loading
            {
                kv_load_more(app);
            }
        }
        Section::R2 => {
            let n = app.r2.entries.len();
            app.r2.entry.move_by(delta, n);
            if n > 0
                && app.r2.entry.sel + 1 >= n
                && app.r2.cursor_token.is_some()
                && !app.r2.loading
            {
                r2_load_more(app);
            }
        }
        Section::Queues => {
            if app.queues.pulled.is_empty() {
                app.queues.scroll = (app.queues.scroll as i64 + delta).max(0) as usize;
            } else {
                app.queues.msg.move_by(delta, app.queues.pulled.len());
            }
        }
        Section::DurableObjects => {
            let n = app.dos.objects.len();
            app.dos.object.move_by(delta, n);
            if n > 0
                && app.dos.object.sel + 1 >= n
                && app.dos.cursor_token.is_some()
                && !app.dos.loading
            {
                do_load_more(app);
            }
        }
    }
}

fn detail_enter(app: &mut App) {
    match app.section {
        Section::Workers => {
            if app.workers.versions_mode {
                deploy_selected_version(app, 100.0);
            } else {
                app.workers.versions_mode = true;
            }
        }
        Section::Pages => {
            if let Some(d) = app.pages.deployments.get(app.pages.dep.sel) {
                let v = serde_json::json!({
                    "id": d.id, "environment": d.environment, "url": d.url, "aliases": d.aliases,
                    "status": d.status(), "created_on": d.created_on, "modified_on": d.modified_on,
                    "trigger": d.deployment_trigger,
                });
                app.open_json_viewer("deployment", &v);
            }
        }
        Section::D1 => match app.d1.view {
            D1View::Tables => {
                if let (Some(db), Some(t)) = (
                    app.d1.list.current().map(|d| d.uuid.clone()),
                    app.d1.tables.get(app.d1.table.sel).map(|t| t.name.clone()),
                ) {
                    app.open_table(db, t);
                }
            }
            D1View::Grid => {
                if let Some(g) = &app.d1.grid
                    && let Some(row) = g.rows.get(app.d1.row.sel)
                {
                    let mut m = serde_json::Map::new();
                    for (i, c) in g.columns.iter().enumerate() {
                        m.insert(
                            c.clone(),
                            row.get(i).cloned().unwrap_or(serde_json::Value::Null),
                        );
                    }
                    let v = serde_json::Value::Object(m);
                    app.open_json_viewer("row", &v);
                }
            }
        },
        Section::Kv => {
            if let (Some(ns), Some(k)) = (
                app.kv.keys_for.clone(),
                app.kv.keys.get(app.kv.key.sel).map(|k| k.name.clone()),
            ) {
                app.send(Command::GetValue { ns, key: k });
            }
        }
        Section::R2 => {
            let Some((bucket, _)) = app.r2.entries_for.clone() else {
                return;
            };
            match app.r2.entries.get(app.r2.entry.sel).cloned() {
                Some(R2Entry::Up) => r2_up(app),
                Some(R2Entry::Folder(f)) => app.load_objects(bucket, f),
                Some(R2Entry::Object(o)) => {
                    if o.size > 8 * 1024 * 1024 {
                        app.toast_info("object is over 8 MB; use s to download it".into());
                    } else {
                        app.send(Command::GetObject { bucket, key: o.key });
                    }
                }
                None => {}
            }
        }
        Section::Queues => {
            if let Some(m) = app.queues.pulled.get(app.queues.msg.sel) {
                let v = serde_json::json!({
                    "id": m.id, "attempts": m.attempts, "timestamp_ms": m.timestamp_ms,
                    "lease_id": m.lease_id, "metadata": m.metadata, "body": decode_body(&m.body),
                });
                app.open_json_viewer("message", &v);
            }
        }
        Section::DurableObjects => {
            if let Some(o) = app.dos.objects.get(app.dos.object.sel) {
                let id = o.id.clone();
                app.copy_text(&id, "object id");
            }
        }
    }
}

/// Pulled bodies for json content come base64-encoded.
fn decode_body(v: &serde_json::Value) -> serde_json::Value {
    if let Some(s) = v.as_str()
        && let Some(bytes) = b64_decode(s)
        && let Ok(text) = String::from_utf8(bytes)
    {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) {
            return j;
        }
        return serde_json::Value::String(text);
    }
    v.clone()
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() || !s.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let mut n: u32 = 0;
        for &c in chunk {
            n = (n << 6) | if c == b'=' { 0 } else { val(c)? };
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

fn kv_load_more(app: &mut App) {
    if let (Some(ns), Some(cur)) = (app.kv.keys_for.clone(), app.kv.cursor_token.clone()) {
        app.kv.loading = true;
        let prefix = app.kv.prefix.clone();
        app.send(Command::ListKeys {
            ns,
            prefix,
            cursor: Some(cur),
        });
    }
}

fn r2_load_more(app: &mut App) {
    if let (Some((bucket, prefix)), Some(cur)) =
        (app.r2.entries_for.clone(), app.r2.cursor_token.clone())
    {
        app.r2.loading = true;
        app.send(Command::ListObjects {
            bucket,
            prefix,
            cursor: Some(cur),
        });
    }
}

fn do_load_more(app: &mut App) {
    if let (Some(ns), Some(cur)) = (app.dos.objects_for.clone(), app.dos.cursor_token.clone()) {
        app.dos.loading = true;
        app.send(Command::ListDoObjects {
            ns,
            cursor: Some(cur),
        });
    }
}

fn r2_up(app: &mut App) {
    let Some((bucket, prefix)) = app.r2.entries_for.clone() else {
        return;
    };
    if prefix.is_empty() {
        app.focus = Focus::List;
        return;
    }
    let trimmed = prefix.trim_end_matches('/');
    let parent = match trimmed.rfind('/') {
        Some(i) => trimmed[..=i].to_string(),
        None => String::new(),
    };
    app.load_objects(bucket, parent);
}

fn deploy_selected_version(app: &mut App, pct: f64) {
    let Some(d) = &app.workers.detail else {
        return;
    };
    let Some(v) = d.versions.get(app.workers.versions.sel) else {
        return;
    };
    let name = d.name.clone();
    let vid = v.id.clone();
    let live: Option<String> = d
        .deployments
        .first()
        .and_then(|dep| {
            dep.versions
                .iter()
                .max_by(|a, b| a.percentage.total_cmp(&b.percentage))
        })
        .map(|dv| dv.version_id.clone());
    let is_live = d
        .deployments
        .first()
        .map(|dep| {
            dep.versions
                .iter()
                .any(|x| x.version_id == vid && x.percentage >= 100.0)
        })
        .unwrap_or(false);
    if pct >= 100.0 {
        if is_live {
            app.toast_info("that version already serves 100% of traffic".into());
            return;
        }
        let label = format!("{}… 100%", &vid[..8.min(vid.len())]);
        let body = vec![
            format!("worker: {name}"),
            format!("version: {} ({})", vid, v.message().unwrap_or("no message")),
            "all traffic moves to this version".into(),
        ];
        app.open_confirm(
            "deploy version",
            body,
            None,
            PendingAction::Deploy {
                name,
                versions: vec![(vid, 100.0)],
                label,
            },
            false,
        );
    } else {
        app.open_prompt(
            "gradual deploy",
            "percentage of traffic for this version (rest stays on the live one)",
            "10",
            false,
            PromptAction::GradualPercent {
                script: name,
                version: vid,
                current: live,
            },
        );
    }
}

/// Section-specific single-key actions (also reachable from the palette).
pub fn section_key(app: &mut App, c: char) {
    match app.section {
        Section::Workers => workers_key(app, c),
        Section::Pages => pages_key(app, c),
        Section::D1 => d1_key(app, c),
        Section::Kv => kv_key(app, c),
        Section::R2 => r2_key(app, c),
        Section::Queues => queues_key(app, c),
        Section::DurableObjects => dos_key(app, c),
    }
}

fn workers_key(app: &mut App, c: char) {
    let Some(name) = app.workers.list.current().map(|s| s.id.clone()) else {
        return;
    };
    match c {
        't' => {
            app.tail = Some(TailState {
                script: name.clone(),
                events: Default::default(),
                sel: None,
                paused: false,
                status: "connecting…".into(),
                filter: String::new(),
                pending: Vec::new(),
                seen: 0,
            });
            app.send(Command::StartTail {
                script: name,
                filters: TailFilters::default(),
            });
        }
        'c' => {
            app.toast_info("downloading bundle…".into());
            app.send(Command::LoadScriptContent { name });
        }
        'v' => {
            app.focus = Focus::Detail;
            app.workers.versions_mode = !app.workers.versions_mode;
        }
        '%' => {
            if app.workers.versions_mode {
                deploy_selected_version(app, 10.0);
            } else {
                app.toast_info("press v to pick a version first".into());
            }
        }
        's' => app.open_prompt(
            &format!("new secret on {name}"),
            "secret name",
            "",
            false,
            PromptAction::SecretName { script: name },
        ),
        'x' => {
            let secrets: Vec<String> = app
                .workers
                .detail
                .as_ref()
                .map(|d| d.secrets.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            if secrets.is_empty() {
                app.toast_info("no secrets on this worker".into());
                return;
            }
            // Pick via a palette-like confirm: user types the exact name.
            app.open_confirm(
                &format!("delete a secret from {name}"),
                vec![
                    format!("secrets: {}", secrets.join(", ")),
                    "type the secret name to delete it".into(),
                ],
                Some(String::new()),
                PendingAction::DeleteSecret {
                    name,
                    key: String::new(),
                },
                true,
            );
        }
        'k' => {
            let current = app
                .workers
                .detail
                .as_ref()
                .map(|d| {
                    d.schedules
                        .iter()
                        .map(|s| s.cron.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            app.open_prompt(
                &format!("cron triggers for {name}"),
                "comma-separated cron expressions (empty removes all)",
                &current,
                false,
                PromptAction::Cron { script: name },
            );
        }
        'w' => {
            let enabled = app
                .workers
                .detail
                .as_ref()
                .and_then(|d| d.subdomain.as_ref())
                .map(|s| s.enabled)
                .unwrap_or(false);
            app.open_confirm(
                if enabled {
                    "disable workers.dev"
                } else {
                    "enable workers.dev"
                },
                vec![format!("worker: {name}")],
                None,
                PendingAction::SetSubdomain {
                    name,
                    enabled: !enabled,
                },
                enabled,
            );
        }
        'y' => {
            let url = app.workers.detail.as_ref().and_then(|d| {
                d.account_subdomain
                    .as_ref()
                    .map(|s| format!("https://{}.{s}.workers.dev", d.name))
            });
            match url {
                Some(u) => app.copy_text(&u, "url"),
                None => app.toast_info("no workers.dev subdomain known yet".into()),
            }
        }
        'D' => app.open_confirm(
            "delete worker",
            vec![
                format!("this deletes {name} and its versions"),
                "type the worker name to confirm".into(),
            ],
            Some(name.clone()),
            PendingAction::DeleteScript { name },
            true,
        ),
        _ => {}
    }
}

fn pages_key(app: &mut App, c: char) {
    let Some(p) = app.pages.list.current().cloned() else {
        return;
    };
    let dep = app.pages.deployments.get(app.pages.dep.sel).cloned();
    match c {
        'l' => {
            if let Some(d) = dep {
                app.send(Command::PagesLogs {
                    project: p.name,
                    id: d.id,
                });
            }
        }
        'R' => {
            let Some(d) = dep else { return };
            if d.environment.as_deref() != Some("production") {
                app.toast_info("only production deployments can be rolled back to".into());
                return;
            }
            app.open_confirm(
                "rollback",
                vec![
                    format!("project: {}", p.name),
                    format!(
                        "production becomes deployment {} ({})",
                        d.short_id.clone().unwrap_or_default(),
                        d.commit_message().unwrap_or("no message")
                    ),
                ],
                None,
                PendingAction::PagesRollback {
                    project: p.name,
                    id: d.id,
                },
                false,
            );
        }
        'b' => {
            let Some(d) = dep else { return };
            app.open_confirm(
                "retry build",
                vec![
                    format!("project: {}", p.name),
                    format!("deployment {}", d.short_id.clone().unwrap_or_default()),
                ],
                None,
                PendingAction::PagesRetry {
                    project: p.name,
                    id: d.id,
                },
                false,
            );
        }
        'y' => {
            let url = dep
                .and_then(|d| d.url)
                .or_else(|| p.subdomain.clone().map(|s| format!("https://{s}")));
            match url {
                Some(u) => app.copy_text(&u, "url"),
                None => app.toast_info("no url".into()),
            }
        }
        'D' => app.open_confirm(
            "delete Pages project",
            vec![
                format!("this deletes {} and all its deployments", p.name),
                "type the project name to confirm".into(),
            ],
            Some(p.name.clone()),
            PendingAction::DeletePagesProject { name: p.name },
            true,
        ),
        _ => {}
    }
}

fn d1_key(app: &mut App, c: char) {
    let Some(db) = app.d1.list.current().cloned() else {
        if c == 'n' {
            app.open_prompt(
                "new D1 database",
                "name",
                "",
                false,
                PromptAction::NewDatabase,
            );
        }
        return;
    };
    match c {
        's' => {
            let initial = if app.d1.last_sql.is_empty() {
                match app.d1.tables.get(app.d1.table.sel) {
                    Some(t) => format!("SELECT * FROM {} LIMIT 100;", d1::quote_ident(&t.name)),
                    None => "SELECT name FROM sqlite_master WHERE type='table';".into(),
                }
            } else {
                app.d1.last_sql.clone()
            };
            app.open_editor(
                &format!("SQL · {}", db.name),
                "Ctrl-S / Ctrl-Enter runs · Esc cancels · writes ask for confirmation",
                &initial,
                EditorAction::Sql { db: db.uuid },
            );
        }
        'n' => app.open_prompt(
            "new D1 database",
            "name",
            "",
            false,
            PromptAction::NewDatabase,
        ),
        'D' => app.open_confirm(
            "delete D1 database",
            vec![
                format!("this permanently deletes {} and all its data", db.name),
                "type the database name to confirm".into(),
            ],
            Some(db.name.clone()),
            PendingAction::DeleteDatabase {
                id: db.uuid,
                name: db.name,
            },
            true,
        ),
        'b' => {
            app.send(Command::D1Bookmark { id: db.uuid });
        }
        'e' | 'i' | 'd' if app.d1.view == D1View::Grid => {
            let Some(table) = app.d1.current_table.clone() else {
                app.toast_info(
                    "row editing works on a table opened with Enter, not on a free query".into(),
                );
                return;
            };
            if app.d1.columns.is_empty() {
                app.toast_info("columns still loading; try again".into());
                return;
            }
            let cols = app.d1.columns.clone();
            let row = app
                .d1
                .grid
                .as_ref()
                .and_then(|g| g.rows.get(app.d1.row.sel).cloned());
            match c {
                'e' => {
                    let Some(row) = row else { return };
                    let text = row_to_json(&cols, &row);
                    app.open_editor(
                        &format!("edit row · {table}"),
                        "Ctrl-S saves (UPDATE keyed by primary key) · Esc cancels",
                        &text,
                        EditorAction::D1EditRow {
                            db: db.uuid,
                            table,
                            cols,
                            old: row,
                        },
                    );
                }
                'i' => {
                    let text = empty_row_json(&cols);
                    app.open_editor(
                        &format!("insert row · {table}"),
                        "Ctrl-S inserts · null on an INTEGER PRIMARY KEY autoassigns",
                        &text,
                        EditorAction::D1InsertRow {
                            db: db.uuid,
                            table,
                            cols,
                        },
                    );
                }
                _ => {
                    let Some(row) = row else { return };
                    let sql = d1::delete_sql(&table, &cols, &row);
                    app.open_confirm(
                        "delete row",
                        vec![util::one_line(&sql, 100)],
                        None,
                        PendingAction::SqlWrite { db: db.uuid, sql },
                        true,
                    );
                }
            }
        }
        _ => {}
    }
}

fn kv_key(app: &mut App, c: char) {
    let ns = app.kv.list.current().cloned();
    match c {
        'N' => app.open_prompt(
            "new KV namespace",
            "title",
            "",
            false,
            PromptAction::NewNamespace,
        ),
        'X' => {
            if let Some(ns) = ns {
                app.open_confirm(
                    "delete KV namespace",
                    vec![
                        format!("this deletes {} and every key in it", ns.title),
                        "type the namespace title to confirm".into(),
                    ],
                    Some(ns.title.clone()),
                    PendingAction::DeleteNamespace {
                        id: ns.id,
                        title: ns.title,
                    },
                    true,
                );
            }
        }
        'n' => {
            if let Some(ns) = ns {
                app.open_prompt(
                    &format!("new key in {}", ns.title),
                    "key name",
                    &app.kv.prefix.clone(),
                    false,
                    PromptAction::NewKey { ns: ns.id },
                );
            }
        }
        'p' => {
            if let Some(ns) = ns {
                let cur = app.kv.prefix.clone();
                app.open_prompt(
                    "filter keys",
                    "prefix",
                    &cur,
                    false,
                    PromptAction::KeyPrefix { ns: ns.id },
                );
            }
        }
        'G' => kv_load_more(app),
        'e' | 'm' | 'd' => {
            let (Some(ns), Some(key)) = (ns, app.kv.keys.get(app.kv.key.sel).cloned()) else {
                return;
            };
            match c {
                'd' => app.open_confirm(
                    "delete key",
                    vec![format!("{} / {}", ns.title, key.name)],
                    None,
                    PendingAction::DeleteKey {
                        ns: ns.id,
                        key: key.name,
                    },
                    true,
                ),
                'e' => {
                    // Fetch the value, then open the editor on arrival.
                    app.pending_edit = Some((ns.id.clone(), key.name.clone()));
                    app.send(Command::GetValue {
                        ns: ns.id,
                        key: key.name,
                    });
                }
                _ => {
                    app.pending_meta_edit = Some((ns.id.clone(), key.name.clone()));
                    app.send(Command::GetValue {
                        ns: ns.id,
                        key: key.name,
                    });
                }
            }
        }
        'D' => {
            let Some(ns) = ns else { return };
            let keys: Vec<String> = app.kv.keys.iter().map(|k| k.name.clone()).collect();
            if keys.is_empty() {
                app.toast_info("no keys loaded".into());
                return;
            }
            let more = app.kv.cursor_token.is_some();
            app.open_confirm(
                "delete loaded keys",
                vec![
                    format!("namespace: {}", ns.title),
                    format!("prefix: \"{}\"", app.kv.prefix),
                    format!(
                        "{} loaded keys will be deleted{}",
                        keys.len(),
                        if more {
                            " (more exist beyond the loaded page: press G to load them first)"
                        } else {
                            ""
                        }
                    ),
                    format!("type {} to confirm", keys.len()),
                ],
                Some(keys.len().to_string()),
                PendingAction::BulkDeleteKeys { ns: ns.id, keys },
                true,
            );
        }
        _ => {}
    }
}

fn r2_key(app: &mut App, c: char) {
    let bucket = app.r2.list.current().cloned();
    match c {
        'N' => app.open_prompt("new R2 bucket", "name", "", false, PromptAction::NewBucket),
        'X' => {
            if let Some(b) = bucket {
                app.open_confirm(
                    "delete R2 bucket",
                    vec![
                        format!("bucket {} (must be empty)", b.name),
                        "type the bucket name to confirm".into(),
                    ],
                    Some(b.name.clone()),
                    PendingAction::DeleteBucket { name: b.name },
                    true,
                );
            }
        }
        'u' => {
            if let Some(b) = bucket {
                let prefix = app.r2.prefix.clone();
                app.open_prompt(
                    &format!("upload to {}/{}", b.name, prefix),
                    "local file path (≤ 300 MB)",
                    "",
                    false,
                    PromptAction::UploadPath {
                        bucket: b.name,
                        prefix,
                    },
                );
            }
        }
        'p' => {
            if let Some(b) = bucket {
                let enabled = app
                    .r2
                    .info
                    .as_ref()
                    .and_then(|i| i.managed.as_ref())
                    .map(|m| m.enabled)
                    .unwrap_or(false);
                app.open_confirm(
                    if enabled {
                        "disable public r2.dev access"
                    } else {
                        "enable public r2.dev access"
                    },
                    vec![
                        format!("bucket: {}", b.name),
                        if enabled {
                            "the pub-….r2.dev URL stops serving".into()
                        } else {
                            "every object becomes readable by anyone with the URL".into()
                        },
                    ],
                    None,
                    PendingAction::SetManagedDomain {
                        bucket: b.name,
                        enabled: !enabled,
                    },
                    !enabled,
                );
            }
        }
        'G' => r2_load_more(app),
        's' | 'd' => {
            let Some(b) = bucket else { return };
            let Some(R2Entry::Object(o)) = app.r2.entries.get(app.r2.entry.sel).cloned() else {
                app.toast_info("select an object".into());
                return;
            };
            if c == 's' {
                let fname = o.key.rsplit('/').next().unwrap_or(&o.key).to_string();
                app.open_prompt(
                    &format!("download {}", o.key),
                    "save to (file or directory)",
                    &format!("./{fname}"),
                    false,
                    PromptAction::DownloadPath {
                        bucket: b.name,
                        key: o.key,
                    },
                );
            } else {
                app.open_confirm(
                    "delete object",
                    vec![format!("{} / {}", b.name, o.key)],
                    None,
                    PendingAction::DeleteObject {
                        bucket: b.name,
                        key: o.key,
                    },
                    true,
                );
            }
        }
        _ => {}
    }
}

fn queues_key(app: &mut App, c: char) {
    let q = app.queues.list.current().cloned();
    match c {
        'n' => app.open_prompt("new queue", "name", "", false, PromptAction::NewQueue),
        _ => {
            let Some(q) = q else { return };
            match c {
                'm' => app.open_editor(
                    &format!("push message to {}", q.queue_name),
                    "Ctrl-S sends · JSON bodies are sent as json, anything else as text",
                    "",
                    EditorAction::PushMessage { id: q.queue_id },
                ),
                'l' => {
                    if !q.has_pull_consumer() {
                        app.toast_err("pulling needs an http_pull consumer on the queue (worker consumers cannot coexist with it)".into());
                        return;
                    }
                    app.open_prompt(
                        &format!("pull from {}", q.queue_name),
                        "batch size (1–100); messages are leased 30 s, ack with a",
                        "10",
                        false,
                        PromptAction::PullBatch { id: q.queue_id },
                    );
                }
                'a' | 'A' => {
                    let leases: Vec<String> = if c == 'A' {
                        app.queues
                            .pulled
                            .iter()
                            .map(|m| m.lease_id.clone())
                            .collect()
                    } else {
                        app.queues
                            .pulled
                            .get(app.queues.msg.sel)
                            .map(|m| vec![m.lease_id.clone()])
                            .unwrap_or_default()
                    };
                    if leases.is_empty() {
                        app.toast_info("nothing pulled".into());
                        return;
                    }
                    app.open_confirm(
                        "acknowledge messages",
                        vec![format!(
                            "{} message(s) will be removed from {}",
                            leases.len(),
                            q.queue_name
                        )],
                        None,
                        PendingAction::AckMessages {
                            id: q.queue_id,
                            acks: leases,
                            retries: Vec::new(),
                        },
                        false,
                    );
                }
                'p' => {
                    let paused = q.paused();
                    app.open_confirm(
                        if paused {
                            "resume delivery"
                        } else {
                            "pause delivery"
                        },
                        vec![format!("queue: {}", q.queue_name)],
                        None,
                        PendingAction::SetQueuePaused {
                            id: q.queue_id,
                            paused: !paused,
                        },
                        !paused,
                    );
                }
                'P' => app.open_confirm(
                    "purge queue",
                    vec![
                        format!("every message in {} is deleted permanently", q.queue_name),
                        "type the queue name to confirm".into(),
                    ],
                    Some(q.queue_name.clone()),
                    PendingAction::PurgeQueue {
                        id: q.queue_id,
                        name: q.queue_name,
                    },
                    true,
                ),
                'D' => app.open_confirm(
                    "delete queue",
                    vec![
                        format!("queue {} and its messages", q.queue_name),
                        "type the queue name to confirm".into(),
                    ],
                    Some(q.queue_name.clone()),
                    PendingAction::DeleteQueue {
                        id: q.queue_id,
                        name: q.queue_name,
                    },
                    true,
                ),
                _ => {}
            }
        }
    }
}

fn dos_key(app: &mut App, c: char) {
    match c {
        'G' => do_load_more(app),
        'y' => {
            if let Some(o) = app.dos.objects.get(app.dos.object.sel) {
                let id = o.id.clone();
                app.copy_text(&id, "object id");
            } else if let Some(n) = app.dos.list.current() {
                let id = n.id.clone();
                app.copy_text(&id, "namespace id");
            }
        }
        _ => {}
    }
}

// ----- tail screen -----------------------------------------------------------

fn on_key_tail(app: &mut App, key: KeyEvent) {
    let Some(t) = &mut app.tail else { return };
    let n = t.visible().len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.tail = None;
            app.send(Command::StopTail);
        }
        KeyCode::Char(' ') | KeyCode::Char('p') => {
            t.paused = !t.paused;
        }
        KeyCode::Char('c') => {
            t.events.clear();
            t.pending.clear();
            t.sel = None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            t.sel = match t.sel {
                None => None,
                Some(s) if s + 1 >= n => None,
                Some(s) => Some(s + 1),
            };
        }
        KeyCode::Char('k') | KeyCode::Up => {
            t.sel = match t.sel {
                None => n.checked_sub(1),
                Some(0) => Some(0),
                Some(s) => Some(s - 1),
            };
        }
        KeyCode::Char('G') | KeyCode::End => t.sel = None,
        KeyCode::Char('g') | KeyCode::Home => t.sel = if n > 0 { Some(0) } else { None },
        KeyCode::Enter | KeyCode::Char('y') => {
            let vis = t.visible();
            let idx = t
                .sel
                .or_else(|| vis.len().checked_sub(1))
                .and_then(|s| vis.get(s).copied());
            if let Some(i) = idx {
                let raw = t.events[i].raw.clone();
                if key.code == KeyCode::Enter {
                    app.open_json_viewer("tail event", &raw);
                } else {
                    let text = serde_json::to_string_pretty(&raw).unwrap_or_default();
                    app.copy_text(&text, "event");
                }
            }
        }
        KeyCode::Char('f') | KeyCode::Char('/') => {
            let cur = t.filter.clone();
            app.open_prompt(
                "filter events",
                "substring (matches request, logs, exceptions)",
                &cur,
                false,
                PromptAction::TailFilter,
            );
        }
        KeyCode::Char('?') => app.open_help(),
        _ => {}
    }
}

// ----- modals ----------------------------------------------------------------

fn on_key_modal(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match &mut app.modal {
        Modal::None => {}
        Modal::Help => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter
            ) {
                app.modal = Modal::None;
            }
        }
        Modal::OpsLog { scroll } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('L') => app.modal = Modal::None,
            KeyCode::Char('j') | KeyCode::Down => *scroll = scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => {
                if *scroll == usize::MAX {
                    *scroll = app.ops_log.len().saturating_sub(2);
                } else {
                    *scroll = scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('G') => *scroll = usize::MAX,
            _ => {}
        },
        Modal::Profiles { items, selected } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.modal = Modal::None,
            KeyCode::Char('j') | KeyCode::Down => {
                *selected = (*selected + 1).min(items.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Enter => {
                let i = *selected;
                app.switch_profile(i);
            }
            _ => {}
        },
        Modal::Accounts { items, selected } => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.modal = Modal::None,
            KeyCode::Char('j') | KeyCode::Down => {
                *selected = (*selected + 1).min(items.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Enter => {
                let i = *selected;
                app.switch_account(i);
            }
            _ => {}
        },
        Modal::Confirm(c) => match key.code {
            KeyCode::Esc => app.modal = Modal::None,
            KeyCode::Enter => {
                // Secret deletion: the typed text is the secret name.
                if let PendingAction::DeleteSecret { key: k, .. } = &mut c.action
                    && k.is_empty()
                {
                    let typed = c.input.text.trim().to_string();
                    let known = app
                        .workers
                        .detail
                        .as_ref()
                        .map(|d| d.secrets.iter().any(|s| s.name == typed))
                        .unwrap_or(false);
                    if !known {
                        return;
                    }
                    *k = typed;
                    c.must_type = None;
                }
                if c.typed_ok() {
                    app.confirm_execute();
                }
            }
            KeyCode::Char('y') if c.must_type.is_none() => app.confirm_execute(),
            KeyCode::Char('n') if c.must_type.is_none() => app.modal = Modal::None,
            _ => {
                if c.must_type.is_some() {
                    c.input.on_key(key);
                }
            }
        },
        Modal::Prompt(p) => match key.code {
            KeyCode::Esc => app.modal = Modal::None,
            KeyCode::Enter => app.submit_prompt(),
            _ => {
                p.input.on_key(key);
            }
        },
        Modal::Editor(ed) => match key.code {
            KeyCode::Esc => app.modal = Modal::None,
            KeyCode::Char('s') if ctrl => app.submit_editor(),
            KeyCode::Enter if ctrl || key.modifiers.contains(KeyModifiers::SHIFT) => {
                app.submit_editor()
            }
            _ => {
                ed.error = None;
                ed.area.on_key(key);
            }
        },
        Modal::Viewer(v) => {
            let page = app.rects.detail.height.max(10) as usize;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => app.modal = Modal::None,
                KeyCode::Char('j') | KeyCode::Down => {
                    v.scroll = (v.scroll + 1).min(v.lines.len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    v.scroll = (v.scroll + page).min(v.lines.len().saturating_sub(1))
                }
                KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(page),
                KeyCode::Char('g') | KeyCode::Home => v.scroll = 0,
                KeyCode::Char('G') | KeyCode::End => v.scroll = v.lines.len().saturating_sub(1),
                KeyCode::Char('h') | KeyCode::Left => v.hscroll = v.hscroll.saturating_sub(8),
                KeyCode::Char('l') | KeyCode::Right => v.hscroll += 8,
                KeyCode::Tab => {
                    // next section
                    if let Some((_, line)) = v.sections.iter().find(|(_, l)| *l > v.scroll) {
                        v.scroll = *line;
                    } else if let Some((_, l)) = v.sections.first() {
                        v.scroll = *l;
                    }
                }
                KeyCode::Char('y') => {
                    let text = v.copy_text.clone();
                    app.copy_text(&text, "contents");
                }
                _ => {}
            }
        }
        Modal::Palette(p) => match key.code {
            KeyCode::Esc => app.modal = Modal::None,
            KeyCode::Down | KeyCode::Tab => {
                p.selected = (p.selected + 1).min(p.filtered.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::BackTab => p.selected = p.selected.saturating_sub(1),
            KeyCode::Char('n') if ctrl => {
                p.selected = (p.selected + 1).min(p.filtered.len().saturating_sub(1))
            }
            KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
            KeyCode::Enter => {
                let action = p.selected_action();
                app.modal = Modal::None;
                if let Some(a) = action {
                    app.run_action(a);
                }
            }
            _ => {
                if p.input.on_key(key) {
                    p.refilter();
                }
            }
        },
    }
}

// ----- mouse -----------------------------------------------------------------

pub fn on_mouse(app: &mut App, m: MouseEvent) {
    if app.modal.is_open() || app.tail.is_some() {
        return;
    }
    let r = app.rects;
    let inside = |rect: ratatui::layout::Rect| {
        m.column >= rect.x
            && m.column < rect.x + rect.width
            && m.row >= rect.y
            && m.row < rect.y + rect.height
    };
    match m.kind {
        MouseEventKind::Down(MouseButton::Left)
            if m.row > r.body.y
                && m.row + 1 < r.body.y + r.body.height
                && (m.column == r.sidebar.x + r.sidebar.width - 1 || m.column == r.list.x) =>
        {
            app.dragging = Some(crate::app::Divider::Sidebar);
        }
        MouseEventKind::Down(MouseButton::Left)
            if m.row > r.body.y
                && m.row + 1 < r.body.y + r.body.height
                && (m.column == r.list.x + r.list.width - 1 || m.column == r.detail.x) =>
        {
            app.dragging = Some(crate::app::Divider::List);
        }
        MouseEventKind::Drag(MouseButton::Left) => match app.dragging {
            Some(crate::app::Divider::Sidebar) => {
                let w = (m.column.saturating_sub(r.body.x) + 1) as i16;
                app.sidebar_w = w.clamp(12, 40) as u16;
            }
            Some(crate::app::Divider::List) => {
                let rest = (r.body.width.saturating_sub(app.sidebar_w)).max(1) as f32;
                let x = m.column.saturating_sub(r.body.x + app.sidebar_w) as f32;
                app.list_pct = ((x / rest) * 100.0).round().clamp(15.0, 85.0) as u16;
            }
            None => {}
        },
        MouseEventKind::Up(MouseButton::Left) => {
            if app.dragging.take().is_some() {
                app.resize_list(0);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if inside(r.sidebar) {
                app.focus = Focus::Sidebar;
                let row = (m.row - r.sidebar.y) as usize;
                if row >= 1 && row - 1 < SECTIONS.len() {
                    app.section = SECTIONS[row - 1];
                    app.ensure_loaded();
                    app.focus = Focus::List;
                    app.load_selection();
                }
            } else if inside(r.list_inner) {
                app.focus = Focus::List;
                let row = (m.row - r.list_inner.y) as usize;
                let scroll = list_scroll(app);
                let target = scroll + row;
                let len = list_len(app);
                if row >= 1 && target - 1 < len {
                    set_list_selected(app, target - 1);
                    app.load_selection();
                }
            } else if inside(r.detail_inner) {
                app.focus = Focus::Detail;
                click_detail(app, (m.row - r.detail_inner.y) as usize);
            }
        }
        MouseEventKind::ScrollDown => {
            if inside(r.list) {
                move_list(app, 3);
            } else if inside(r.detail) {
                move_detail(app, 3);
            } else if inside(r.sidebar) {
                on_key_sidebar(app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
            }
        }
        MouseEventKind::ScrollUp => {
            if inside(r.list) {
                move_list(app, -3);
            } else if inside(r.detail) {
                move_detail(app, -3);
            } else if inside(r.sidebar) {
                on_key_sidebar(app, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
            }
        }
        _ => {}
    }
}

fn list_scroll(app: &App) -> usize {
    match app.section {
        Section::Workers => app.workers.list.scroll,
        Section::Pages => app.pages.list.scroll,
        Section::D1 => app.d1.list.scroll,
        Section::Kv => app.kv.list.scroll,
        Section::R2 => app.r2.list.scroll,
        Section::Queues => app.queues.list.scroll,
        Section::DurableObjects => app.dos.list.scroll,
    }
}

fn set_list_selected(app: &mut App, i: usize) {
    match app.section {
        Section::Workers => app.workers.list.selected = i,
        Section::Pages => app.pages.list.selected = i,
        Section::D1 => app.d1.list.selected = i,
        Section::Kv => app.kv.list.selected = i,
        Section::R2 => app.r2.list.selected = i,
        Section::Queues => app.queues.list.selected = i,
        Section::DurableObjects => app.dos.list.selected = i,
    }
}

/// Detail panes that are lists: translate a clicked row to a selection.
fn click_detail(app: &mut App, row: usize) {
    let header = 1usize; // list panes render one header line
    match app.section {
        Section::Pages => {
            let i = app.pages.dep.scroll + row.saturating_sub(header);
            if row >= header && i < app.pages.deployments.len() {
                app.pages.dep.sel = i;
            }
        }
        Section::D1 => match app.d1.view {
            D1View::Tables => {
                let i = app.d1.table.scroll + row.saturating_sub(header);
                if row >= header && i < app.d1.tables.len() {
                    app.d1.table.sel = i;
                }
            }
            D1View::Grid => {
                let i = app.d1.row.scroll + row.saturating_sub(2);
                if row >= 2
                    && let Some(g) = &app.d1.grid
                    && i < g.rows.len()
                {
                    app.d1.row.sel = i;
                }
            }
        },
        Section::Kv => {
            let i = app.kv.key.scroll + row.saturating_sub(header);
            if row >= header && i < app.kv.keys.len() {
                app.kv.key.sel = i;
            }
        }
        Section::R2 => {
            let i = app.r2.entry.scroll + row.saturating_sub(header);
            if row >= header && i < app.r2.entries.len() {
                app.r2.entry.sel = i;
            }
        }
        Section::DurableObjects => {
            let i = app.dos.object.scroll + row.saturating_sub(header);
            if row >= header && i < app.dos.objects.len() {
                app.dos.object.sel = i;
            }
        }
        _ => {}
    }
}
