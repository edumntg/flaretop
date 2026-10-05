//! Opening modals and executing what they submit.

use flaretop_core::actor::sql_is_write;
use flaretop_core::products::d1;
use flaretop_core::types::Command;
use ratatui::text::Line;
use serde_json::Value;

use crate::app::{App, D1View, SECTIONS, Section};
use crate::input::Input;
use crate::json_view;
use crate::modal::{
    AppAction, Confirm, Editor, EditorAction, Modal, Palette, PendingAction, Prompt, PromptAction,
    Viewer,
};
use crate::textarea::TextArea;
use crate::util;

impl App {
    pub fn open_prompt(
        &mut self,
        title: &str,
        label: &str,
        initial: &str,
        secret: bool,
        action: PromptAction,
    ) {
        self.modal = Modal::Prompt(Prompt {
            title: title.into(),
            label: label.into(),
            input: Input::with_text(initial),
            secret,
            action,
            error: None,
        });
    }

    pub fn open_confirm(
        &mut self,
        title: &str,
        body: Vec<String>,
        must_type: Option<String>,
        action: PendingAction,
        danger: bool,
    ) {
        if self.read_only {
            self.toast_err("read-only mode: writes are blocked".into());
            return;
        }
        self.modal = Modal::Confirm(Confirm {
            title: title.into(),
            body,
            must_type,
            input: Input::default(),
            action,
            danger,
        });
    }

    pub fn open_editor(&mut self, title: &str, hint: &str, text: &str, action: EditorAction) {
        self.modal = Modal::Editor(Editor {
            title: title.into(),
            hint: hint.into(),
            area: TextArea::from_text(text),
            action,
            error: None,
        });
    }

    pub fn open_viewer(&mut self, title: &str, lines: Vec<Line<'static>>, copy_text: String) {
        self.modal = Modal::Viewer(Viewer::new(title, lines, copy_text));
    }

    pub fn open_code_viewer(&mut self, name: String, parts: Vec<(String, String)>) {
        let mut lines = Vec::new();
        let mut sections = Vec::new();
        let mut copy = String::new();
        for (pname, content) in &parts {
            sections.push((pname.clone(), lines.len()));
            let is_binary = content.bytes().take(512).any(|b| b == 0);
            lines.push(Line::styled(
                format!(
                    "── {pname} ({}) ──",
                    flaretop_core::human_bytes(content.len() as u64)
                ),
                ratatui::style::Style::new().fg(crate::theme::accent()),
            ));
            if is_binary {
                lines.push(Line::styled(
                    "(binary module)".to_string(),
                    ratatui::style::Style::new().fg(crate::theme::dim()),
                ));
            } else {
                lines.extend(json_view::text_lines(content));
                copy.push_str(content);
                copy.push('\n');
            }
            lines.push(Line::raw(""));
        }
        let mut v = Viewer::new(
            format!("{name} — code ({} parts)", parts.len()),
            lines,
            copy,
        );
        v.sections = sections;
        self.modal = Modal::Viewer(v);
    }

    pub fn open_value_viewer(
        &mut self,
        key: String,
        bytes: Vec<u8>,
        expiration: Option<u64>,
        metadata: Option<Value>,
    ) {
        let (text, is_text) = util::bytes_to_text(&bytes);
        let mut lines = Vec::new();
        let mut head = format!("{} bytes", bytes.len());
        if let Some(e) = expiration {
            head.push_str(&format!(
                "  expires {}",
                flaretop_core::auth::civil_from_unix(e as i64)
            ));
        }
        lines.push(Line::styled(
            head,
            ratatui::style::Style::new().fg(crate::theme::dim()),
        ));
        if let Some(m) = &metadata {
            lines.push(Line::styled(
                "metadata:".to_string(),
                ratatui::style::Style::new().fg(crate::theme::accent()),
            ));
            lines.extend(json_view::json_lines(m));
        }
        lines.push(Line::raw(""));
        if is_text {
            lines.extend(json_view::auto_lines(&text));
        } else {
            lines.extend(json_view::text_lines(&text));
        }
        self.open_viewer(&format!("KV {key}"), lines, text);
    }

    pub fn open_object_viewer(
        &mut self,
        key: String,
        bytes: Vec<u8>,
        content_type: Option<String>,
    ) {
        let (text, is_text) = util::bytes_to_text(&bytes);
        let mut lines = vec![Line::styled(
            format!(
                "{}  {}",
                flaretop_core::human_bytes(bytes.len() as u64),
                content_type.unwrap_or_default()
            ),
            ratatui::style::Style::new().fg(crate::theme::dim()),
        )];
        lines.push(Line::raw(""));
        if is_text {
            lines.extend(json_view::auto_lines(&text));
        } else {
            lines.extend(json_view::text_lines(&text));
        }
        self.open_viewer(&format!("R2 {key}"), lines, text);
    }

    pub fn open_json_viewer(&mut self, title: &str, v: &Value) {
        let text = serde_json::to_string_pretty(v).unwrap_or_default();
        self.open_viewer(title, json_view::json_lines(v), text);
    }

    pub fn open_help(&mut self) {
        self.modal = Modal::Help;
    }

    pub fn open_palette(&mut self) {
        let mut actions: Vec<(String, AppAction)> = Vec::new();
        for (i, s) in SECTIONS.iter().enumerate() {
            actions.push((
                format!("go: {} ({})", s.label(), i + 1),
                AppAction::GoSection(i),
            ));
        }
        actions.push(("refresh (r)".into(), AppAction::Refresh));
        actions.push(("help (?)".into(), AppAction::Help));
        actions.push(("operations log (L)".into(), AppAction::OpsLog));
        actions.push(("switch account (A)".into(), AppAction::Accounts));
        actions.push(("switch profile / token (P)".into(), AppAction::Profiles));
        actions.push(("copy equivalent cf command (C)".into(), AppAction::CopyCf));
        actions.push((
            if self.read_only {
                "read-only: turn off".into()
            } else {
                "read-only: turn on".into()
            },
            AppAction::ToggleReadOnly,
        ));
        actions.push((
            "token: save current token to config".into(),
            AppAction::SaveToken,
        ));
        actions.push(("token: forget saved token".into(), AppAction::ForgetToken));
        for name in crate::theme::NAMES {
            actions.push((format!("theme: {name}"), AppAction::SetTheme(name)));
        }
        let section_actions: &[(&str, char)] = match self.section {
            Section::Workers => &[
                ("worker: live tail (t)", 't'),
                ("worker: view code (c)", 'c'),
                ("worker: versions / deploy (v)", 'v'),
                ("worker: set secret (s)", 's'),
                ("worker: delete secret (x)", 'x'),
                ("worker: edit cron triggers (k)", 'k'),
                ("worker: toggle workers.dev (w)", 'w'),
                ("worker: copy workers.dev url (y)", 'y'),
                ("worker: delete (D)", 'D'),
            ],
            Section::Pages => &[
                ("pages: build log (l)", 'l'),
                ("pages: rollback to deployment (R)", 'R'),
                ("pages: retry build (b)", 'b'),
                ("pages: copy deployment url (y)", 'y'),
                ("pages: delete project (D)", 'D'),
            ],
            Section::D1 => &[
                ("d1: sql console (s)", 's'),
                ("d1: new database (n)", 'n'),
                ("d1: delete database (D)", 'D'),
                ("d1: time travel bookmark (b)", 'b'),
                ("d1: edit row (e)", 'e'),
                ("d1: insert row (i)", 'i'),
                ("d1: delete row (d)", 'd'),
            ],
            Section::Kv => &[
                ("kv: new key (n)", 'n'),
                ("kv: edit value (e)", 'e'),
                ("kv: edit metadata (m)", 'm'),
                ("kv: delete key (d)", 'd'),
                ("kv: delete keys by prefix (D)", 'D'),
                ("kv: filter by prefix (p)", 'p'),
                ("kv: load more (G)", 'G'),
                ("kv: new namespace (N)", 'N'),
                ("kv: delete namespace (X)", 'X'),
            ],
            Section::R2 => &[
                ("r2: download object (s)", 's'),
                ("r2: upload file (u)", 'u'),
                ("r2: delete object (d)", 'd'),
                ("r2: toggle r2.dev public access (p)", 'p'),
                ("r2: load more (G)", 'G'),
                ("r2: new bucket (N)", 'N'),
                ("r2: delete bucket (X)", 'X'),
            ],
            Section::Queues => &[
                ("queue: push message (m)", 'm'),
                ("queue: pull messages (l)", 'l'),
                ("queue: ack selected (a)", 'a'),
                ("queue: pause / resume (p)", 'p'),
                ("queue: purge (P)", 'P'),
                ("queue: new (n)", 'n'),
                ("queue: delete (D)", 'D'),
            ],
            Section::DurableObjects => &[
                ("do: load more objects (G)", 'G'),
                ("do: copy object id (y)", 'y'),
            ],
        };
        for (label, key) in section_actions {
            actions.push(((*label).to_string(), AppAction::Key(*key)));
        }
        actions.push(("quit (q)".into(), AppAction::Quit));
        self.modal = Modal::Palette(Palette::new("command palette", actions));
    }

    pub fn run_action(&mut self, action: AppAction) {
        match action {
            AppAction::GoSection(i) => {
                self.section = SECTIONS[i];
                self.focus = crate::app::Focus::List;
                self.ensure_loaded();
                self.load_selection();
            }
            AppAction::Refresh => self.refresh(),
            AppAction::Help => self.open_help(),
            AppAction::OpsLog => self.modal = Modal::OpsLog { scroll: usize::MAX },
            AppAction::Accounts => {
                let selected = self
                    .accounts
                    .iter()
                    .position(|a| a.id == self.account.id)
                    .unwrap_or(0);
                self.modal = Modal::Accounts {
                    items: self.accounts.clone(),
                    selected,
                };
            }
            AppAction::Profiles => {
                let items = flaretop_core::auth::profiles();
                let selected = self
                    .profile
                    .as_ref()
                    .and_then(|p| items.iter().position(|(n, _)| n == p))
                    .unwrap_or(0);
                self.modal = Modal::Profiles { items, selected };
            }
            AppAction::ToggleReadOnly => {
                self.read_only = !self.read_only;
                self.toast_info(format!(
                    "read-only {}",
                    if self.read_only { "on" } else { "off" }
                ));
            }
            AppAction::Quit => self.should_quit = true,
            AppAction::SetTheme(name) => {
                crate::theme::set_by_name(name);
                self.config.theme = Some(name.to_string());
                let _ = crate::config::save(&self.config);
            }
            AppAction::SaveToken => {
                self.open_prompt(
                    "save token",
                    "paste the API token to store in config (0600)",
                    "",
                    true,
                    PromptAction::SaveToken,
                );
            }
            AppAction::ForgetToken => match flaretop_core::auth::forget_token() {
                Ok(()) => self.toast_info("saved token removed".into()),
                Err(e) => self.toast_err(e.to_string()),
            },
            AppAction::CopyCf => self.copy_cf(),
            AppAction::Key(c) => crate::keys::section_key(self, c),
        }
    }

    pub fn copy_cf(&mut self) {
        match crate::cfcmd::for_context(self) {
            Some(cmd) => {
                self.last_cf = Some(cmd.clone());
                match util::clipboard_copy(&cmd) {
                    Ok(()) => self.toast_info(format!("copied: {cmd}")),
                    Err(e) => self.toast_err(format!("{cmd}  (clipboard: {e})")),
                }
            }
            None => self.toast_info("nothing selected".into()),
        }
    }

    pub fn copy_text(&mut self, text: &str, what: &str) {
        match util::clipboard_copy(text) {
            Ok(()) => self.toast_info(format!("copied {what}")),
            Err(e) => self.toast_err(format!("clipboard: {e}")),
        }
    }

    pub fn switch_account(&mut self, idx: usize) {
        if let Some(a) = self.accounts.get(idx).cloned()
            && a.id != self.account.id
        {
            self.account = a.clone();
            self.send(Command::SetAccount(a.id.clone()));
            self.config.account_id = Some(a.id);
            let _ = crate::config::save(&self.config);
            self.workers = Default::default();
            self.d1 = Default::default();
            self.kv = Default::default();
            self.r2 = Default::default();
            self.queues = Default::default();
            self.dos = Default::default();
            self.ensure_loaded();
        }
        self.modal = Modal::None;
    }

    /// Switch to a saved profile: new client, new actor, fresh state.
    pub fn switch_profile(&mut self, idx: usize) {
        let items = flaretop_core::auth::profiles();
        let Some((name, p)) = items.get(idx).cloned() else {
            self.modal = Modal::None;
            return;
        };
        self.modal = Modal::None;
        if self.profile.as_deref() == Some(&name) {
            return;
        }
        let client = match flaretop_core::api::Client::new(&p.api_token) {
            Ok(c) => c,
            Err(e) => {
                self.toast_err(e.to_string());
                return;
            }
        };
        if self.tail.is_some() {
            self.tail = None;
            self.send(Command::StopTail);
        }
        let account_id = p
            .account_id
            .clone()
            .unwrap_or_else(|| self.account.id.clone());
        self.handle =
            flaretop_core::actor::spawn(client.clone(), account_id.clone(), self.read_only);
        self.client = client;
        self.profile = Some(name.clone());
        self.cred_source = flaretop_core::auth::CredSource::ConfigFile;
        self.cred_expires = None;
        self.account = flaretop_core::auth::Account {
            id: account_id,
            name: "account".into(),
        };
        self.accounts.clear();
        self.config.profile = Some(name.clone());
        let _ = crate::config::save(&self.config);
        self.workers = Default::default();
        self.pages = Default::default();
        self.d1 = Default::default();
        self.kv = Default::default();
        self.r2 = Default::default();
        self.queues = Default::default();
        self.dos = Default::default();
        self.inflight = 0;
        self.send(Command::ListAccounts);
        self.ensure_loaded();
        self.toast_info(format!("profile {name}"));
    }

    // ----- prompt submission -------------------------------------------------

    pub fn submit_prompt(&mut self) {
        let Modal::Prompt(p) = std::mem::replace(&mut self.modal, Modal::None) else {
            return;
        };
        let text = p.input.text.trim().to_string();
        match p.action {
            PromptAction::SecretName { script } => {
                if text.is_empty() {
                    return;
                }
                self.open_prompt(
                    &format!("secret {text} on {script}"),
                    "value (hidden)",
                    "",
                    true,
                    PromptAction::SecretValue { script, key: text },
                );
            }
            PromptAction::SecretValue { script, key } => {
                if self.read_only {
                    self.toast_err("read-only mode".into());
                    return;
                }
                self.send(Command::PutSecret {
                    name: script,
                    key,
                    value: p.input.text,
                });
            }
            PromptAction::Cron { script } => {
                let crons: Vec<String> = text
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.open_confirm(
                    "set cron triggers",
                    vec![
                        format!("worker: {script}"),
                        if crons.is_empty() {
                            "remove all cron triggers".into()
                        } else {
                            format!("triggers: {}", crons.join(" | "))
                        },
                    ],
                    None,
                    PendingAction::Deploy {
                        name: script.clone(),
                        versions: Vec::new(),
                        label: format!("cron:{}", crons.join(",")),
                    },
                    false,
                );
                // Deploy with an empty version list is the marker for "put schedules".
            }
            PromptAction::GradualPercent {
                script,
                version,
                current,
            } => {
                let pct: f64 = match text.trim_end_matches('%').parse() {
                    Ok(v) if (0.0..=100.0).contains(&v) => v,
                    _ => {
                        self.toast_err("percentage must be 0–100".into());
                        return;
                    }
                };
                let mut versions = vec![(version.clone(), pct)];
                if pct < 100.0 {
                    match current {
                        Some(cur) if cur != version => versions.push((cur, 100.0 - pct)),
                        _ => {
                            self.toast_err("no other live version to split traffic with".into());
                            return;
                        }
                    }
                }
                let label = versions
                    .iter()
                    .map(|(id, p)| format!("{}… {p}%", &id[..8.min(id.len())]))
                    .collect::<Vec<_>>()
                    .join(" + ");
                self.open_confirm(
                    "deploy",
                    vec![format!("worker: {script}"), format!("traffic: {label}")],
                    None,
                    PendingAction::Deploy {
                        name: script,
                        versions,
                        label,
                    },
                    false,
                );
            }
            PromptAction::NewDatabase => {
                if !text.is_empty() {
                    self.send(Command::CreateDatabase { name: text });
                }
            }
            PromptAction::NewNamespace => {
                if !text.is_empty() {
                    self.send(Command::CreateNamespace { title: text });
                }
            }
            PromptAction::NewKey { ns } => {
                if text.is_empty() {
                    return;
                }
                self.open_editor(
                    &format!("new key {text}"),
                    "Ctrl-S to save · Esc to cancel",
                    "",
                    EditorAction::KvPut {
                        ns,
                        key: text,
                        metadata: None,
                    },
                );
            }
            PromptAction::KeyPrefix { ns } => {
                self.load_keys(ns, text);
            }
            PromptAction::KeyTtl {
                ns,
                key,
                value,
                metadata,
            } => {
                let ttl = if text.is_empty() {
                    None
                } else {
                    match text.parse::<u64>() {
                        Ok(t) if t >= 60 => Some(t),
                        _ => {
                            self.toast_err(
                                "TTL must be a number of seconds ≥ 60 (or empty)".into(),
                            );
                            return;
                        }
                    }
                };
                self.send(Command::PutValue {
                    ns,
                    key,
                    value,
                    metadata,
                    ttl,
                });
            }
            PromptAction::NewBucket => {
                if !text.is_empty() {
                    self.send(Command::CreateBucket { name: text });
                }
            }
            PromptAction::DownloadPath { bucket, key } => {
                if text.is_empty() {
                    return;
                }
                self.pending_download = Some(expand_path(&text));
                self.send(Command::GetObject { bucket, key });
            }
            PromptAction::UploadPath { bucket, prefix } => {
                if text.is_empty() {
                    return;
                }
                let path = expand_path(&text);
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        if bytes.len() > 300 * 1024 * 1024 {
                            self.toast_err("objects over 300 MB need the S3 API".into());
                            return;
                        }
                        let fname = std::path::Path::new(&path)
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_else(|| "upload".into());
                        let key = format!("{prefix}{fname}");
                        let content_type = util::guess_content_type(&path).to_string();
                        self.send(Command::PutObject {
                            bucket,
                            key,
                            bytes,
                            content_type,
                        });
                    }
                    Err(e) => self.toast_err(format!("{path}: {e}")),
                }
            }
            PromptAction::NewQueue => {
                if !text.is_empty() {
                    self.send(Command::CreateQueue { name: text });
                }
            }
            PromptAction::PullBatch { id } => {
                let batch: usize = text.parse().unwrap_or(10);
                self.send(Command::PullMessages {
                    id,
                    batch: batch.clamp(1, 100),
                });
            }
            PromptAction::SaveToken => match flaretop_core::auth::save_token(&p.input.text) {
                Ok(path) => self.toast_info(format!("saved to {}", path.display())),
                Err(e) => self.toast_err(e.to_string()),
            },
            PromptAction::TailFilter => {
                if let Some(t) = &mut self.tail {
                    t.filter = text;
                    t.sel = None;
                }
            }
        }
    }

    // ----- editor submission -------------------------------------------------

    pub fn submit_editor(&mut self) {
        let Modal::Editor(ed) = &mut self.modal else {
            return;
        };
        let text = ed.area.text();
        let action = ed.action.clone();
        match action {
            EditorAction::Sql { db } => {
                let sql = text.trim().to_string();
                if sql.is_empty() {
                    return;
                }
                self.modal = Modal::None;
                self.d1.current_table = None;
                if sql_is_write(&sql) {
                    self.open_confirm(
                        "run write statement",
                        vec![util::one_line(&sql, 100)],
                        None,
                        PendingAction::SqlWrite { db, sql },
                        true,
                    );
                } else {
                    self.run_sql(db, sql);
                }
            }
            EditorAction::KvPut { ns, key, metadata } => {
                self.modal = Modal::None;
                self.open_prompt(
                    &format!("put {key}"),
                    "TTL in seconds (empty = never expires)",
                    "",
                    false,
                    PromptAction::KeyTtl {
                        ns,
                        key,
                        value: text.into_bytes(),
                        metadata,
                    },
                );
            }
            EditorAction::KvMetadata { ns, key, value } => {
                let meta = if text.trim().is_empty() {
                    None
                } else {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            ed.error = Some(format!("invalid JSON: {e}"));
                            return;
                        }
                    }
                };
                self.modal = Modal::None;
                self.send(Command::PutValue {
                    ns,
                    key,
                    value,
                    metadata: meta,
                    ttl: None,
                });
            }
            EditorAction::PushMessage { id } => {
                let body = text.trim().to_string();
                if body.is_empty() {
                    return;
                }
                let json = serde_json::from_str::<Value>(&body).is_ok();
                self.modal = Modal::None;
                self.send(Command::PushMessage { id, body, json });
            }
            EditorAction::D1EditRow {
                db,
                table,
                cols,
                old,
            } => {
                let new: Vec<Value> = match parse_row(&text, &cols) {
                    Ok(v) => v,
                    Err(e) => {
                        ed.error = Some(e);
                        return;
                    }
                };
                match d1::update_sql(&table, &cols, &old, &new) {
                    Some(sql) => {
                        self.modal = Modal::None;
                        self.open_confirm(
                            "update row",
                            vec![util::one_line(&sql, 100)],
                            None,
                            PendingAction::SqlWrite { db, sql },
                            true,
                        );
                    }
                    None => {
                        self.modal = Modal::None;
                        self.toast_info("no changes".into());
                    }
                }
            }
            EditorAction::D1InsertRow { db, table, cols } => {
                let row: Vec<Value> = match parse_row(&text, &cols) {
                    Ok(v) => v,
                    Err(e) => {
                        ed.error = Some(e);
                        return;
                    }
                };
                let sql = d1::insert_sql(&table, &cols, &row);
                self.modal = Modal::None;
                self.open_confirm(
                    "insert row",
                    vec![util::one_line(&sql, 100)],
                    None,
                    PendingAction::SqlWrite { db, sql },
                    true,
                );
            }
        }
    }

    // ----- confirm execution -------------------------------------------------

    pub fn confirm_execute(&mut self) {
        let Modal::Confirm(c) = std::mem::replace(&mut self.modal, Modal::None) else {
            return;
        };
        if !c.typed_ok() {
            self.modal = Modal::Confirm(c);
            return;
        }
        match c.action {
            PendingAction::DeleteScript { name } => {
                self.send(Command::DeleteScript { name });
            }
            PendingAction::PagesRollback { project, id } => {
                self.send(Command::PagesRollback { project, id });
            }
            PendingAction::PagesRetry { project, id } => {
                self.send(Command::PagesRetry { project, id });
            }
            PendingAction::DeletePagesProject { name } => {
                self.send(Command::DeletePagesProject { name });
                self.pages.deployments_for = None;
            }
            PendingAction::DeleteSecret { name, key } => {
                self.send(Command::DeleteSecret { name, key });
            }
            PendingAction::SetSubdomain { name, enabled } => {
                let previews = self
                    .workers
                    .detail
                    .as_ref()
                    .and_then(|d| d.subdomain.as_ref())
                    .map(|s| s.previews_enabled)
                    .unwrap_or(false);
                self.send(Command::SetSubdomain {
                    name,
                    enabled,
                    previews,
                });
            }
            PendingAction::Deploy {
                name,
                versions,
                label,
            } => {
                if versions.is_empty() {
                    let crons: Vec<String> = label
                        .strip_prefix("cron:")
                        .unwrap_or("")
                        .split(',')
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                        .collect();
                    self.send(Command::PutSchedules { name, crons });
                } else {
                    self.send(Command::Deploy {
                        name,
                        versions,
                        message: Some(format!("flaretop: {label}")),
                    });
                }
            }
            PendingAction::DeleteDatabase { id, name } => {
                self.send(Command::DeleteDatabase { id, name });
                self.d1.tables_for = None;
            }
            PendingAction::SqlWrite { db, sql } => {
                self.run_sql(db, sql);
            }
            PendingAction::DeleteKey { ns, key } => {
                self.send(Command::DeleteKey { ns, key });
            }
            PendingAction::BulkDeleteKeys { ns, keys } => {
                self.send(Command::BulkDeleteKeys { ns, keys });
            }
            PendingAction::DeleteNamespace { id, title } => {
                self.send(Command::DeleteNamespace { id, title });
                self.kv.keys_for = None;
            }
            PendingAction::DeleteObject { bucket, key } => {
                self.send(Command::DeleteObject { bucket, key });
            }
            PendingAction::DeleteBucket { name } => {
                self.send(Command::DeleteBucket { name });
                self.r2.entries_for = None;
            }
            PendingAction::SetManagedDomain { bucket, enabled } => {
                self.send(Command::SetManagedDomain { bucket, enabled });
            }
            PendingAction::PurgeQueue { id, name } => {
                self.send(Command::PurgeQueue { id, name });
            }
            PendingAction::DeleteQueue { id, name } => {
                self.send(Command::DeleteQueue { id, name });
                self.queues.pulled_for = None;
            }
            PendingAction::SetQueuePaused { id, paused } => {
                self.send(Command::SetQueuePaused { id, paused });
            }
            PendingAction::AckMessages { id, acks, retries } => {
                let set: std::collections::HashSet<String> =
                    acks.iter().chain(retries.iter()).cloned().collect();
                self.queues.pulled.retain(|m| !set.contains(&m.lease_id));
                self.queues.msg.clamp(self.queues.pulled.len());
                self.send(Command::AckMessages { id, acks, retries });
            }
        }
    }

    /// A downloaded object arrived while a path was pending: write it.
    pub fn finish_download(&mut self, key: &str, bytes: &[u8]) -> bool {
        let Some(path) = self.pending_download.take() else {
            return false;
        };
        let path = if std::path::Path::new(&path).is_dir() {
            let fname = key.rsplit('/').next().unwrap_or(key);
            format!("{}/{fname}", path.trim_end_matches('/'))
        } else {
            path
        };
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.log(format!("downloaded {key} to {path}"));
                self.toast_info(format!(
                    "saved {} to {path}",
                    flaretop_core::human_bytes(bytes.len() as u64)
                ));
            }
            Err(e) => self.toast_err(format!("{path}: {e}")),
        }
        true
    }

    pub fn d1_back_to_tables(&mut self) {
        self.d1.view = D1View::Tables;
    }
}

fn expand_path(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().to_string();
    }
    p.to_string()
}

/// Parse a JSON object (or array) typed in the row editor into column order.
fn parse_row(text: &str, cols: &[d1::Column]) -> Result<Vec<Value>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    match v {
        Value::Object(m) => {
            for k in m.keys() {
                if !cols.iter().any(|c| &c.name == k) {
                    return Err(format!("unknown column: {k}"));
                }
            }
            Ok(cols
                .iter()
                .map(|c| m.get(&c.name).cloned().unwrap_or(Value::Null))
                .collect())
        }
        Value::Array(a) if a.len() == cols.len() => Ok(a),
        _ => Err("expected a JSON object keyed by column name".into()),
    }
}

/// Row as a JSON object text for editing.
pub fn row_to_json(cols: &[d1::Column], row: &[Value]) -> String {
    let mut m = serde_json::Map::new();
    for (i, c) in cols.iter().enumerate() {
        m.insert(c.name.clone(), row.get(i).cloned().unwrap_or(Value::Null));
    }
    serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default()
}

pub fn empty_row_json(cols: &[d1::Column]) -> String {
    let mut m = serde_json::Map::new();
    for c in cols {
        let v = if c.pk && c.kind.to_ascii_uppercase().contains("INT") {
            Value::Null
        } else if c.kind.to_ascii_uppercase().contains("INT")
            || c.kind.to_ascii_uppercase().contains("REAL")
        {
            Value::from(0)
        } else {
            Value::String(String::new())
        };
        m.insert(c.name.clone(), v);
    }
    serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default()
}
