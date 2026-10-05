//! Application state and the core-event side of the loop. Key handling is
//! in `keys.rs`, prompt/editor/confirm execution in `actions.rs`, drawing
//! in `ui.rs`.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use flaretop_core::actor::Handle;
use flaretop_core::api::{ApiError, Client};
use flaretop_core::auth::{Account, CredSource, Credential};
use flaretop_core::products::{d1, durable_objects as dos, kv, pages, queues, r2, workers};
use flaretop_core::tail::TailEvent;
use flaretop_core::types::{BucketInfo, Command, CoreEvent, Payload, ScriptDetail};
use ratatui::layout::Rect;
use serde_json::Value;

use crate::modal::Modal;
use crate::util::clock_utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Workers,
    Pages,
    D1,
    Kv,
    R2,
    Queues,
    DurableObjects,
}

pub const SECTIONS: [Section; 7] = [
    Section::Workers,
    Section::Pages,
    Section::D1,
    Section::Kv,
    Section::R2,
    Section::Queues,
    Section::DurableObjects,
];

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Workers => "Workers",
            Section::Pages => "Pages",
            Section::D1 => "D1",
            Section::Kv => "KV",
            Section::R2 => "R2",
            Section::Queues => "Queues",
            Section::DurableObjects => "Durable Objects",
        }
    }
    pub fn index(self) -> usize {
        SECTIONS.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    List,
    Detail,
}

/// A filterable, scrollable list of items with one selected row.
pub struct ListState<T> {
    pub items: Vec<T>,
    pub selected: usize,
    pub scroll: usize,
    pub filter: String,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// Indices into `items` matching `filter` (all when filter is empty).
    pub visible: Vec<usize>,
}

impl<T> Default for ListState<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            filter: String::new(),
            loaded: false,
            loading: false,
            error: None,
            visible: Vec::new(),
        }
    }
}

impl<T> ListState<T> {
    pub fn set(&mut self, items: Vec<T>, label: impl Fn(&T) -> String) {
        self.items = items;
        self.loaded = true;
        self.loading = false;
        self.error = None;
        self.refilter(label);
    }
    pub fn refilter(&mut self, label: impl Fn(&T) -> String) {
        let needle = self.filter.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| needle.is_empty() || label(it).to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }
    pub fn current(&self) -> Option<&T> {
        self.visible.get(self.selected).map(|&i| &self.items[i])
    }
    pub fn len(&self) -> usize {
        self.visible.len()
    }
    pub fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let n = self.visible.len() as i64;
        self.selected = (self.selected as i64 + delta).clamp(0, n - 1) as usize;
    }
}

/// Cursor/scroll for a plain detail list (keys, objects, rows).
#[derive(Default, Clone)]
pub struct Cursor {
    pub sel: usize,
    pub scroll: usize,
}

impl Cursor {
    pub fn move_by(&mut self, delta: i64, len: usize) {
        if len == 0 {
            self.sel = 0;
            return;
        }
        self.sel = (self.sel as i64 + delta).clamp(0, len as i64 - 1) as usize;
    }
    pub fn clamp(&mut self, len: usize) {
        self.sel = self.sel.min(len.saturating_sub(1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Sidebar,
    List,
}

#[derive(Default)]
pub struct WorkersState {
    pub list: ListState<workers::Script>,
    pub detail: Option<ScriptDetail>,
    pub detail_for: String,
    pub detail_loading: bool,
    pub scroll: usize,
    /// When true the detail pane is the versions list with its own cursor.
    pub versions_mode: bool,
    pub versions: Cursor,
}

pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub widths: Vec<usize>,
    pub meta: d1::Meta,
    pub sql: String,
}

impl Grid {
    pub fn from_results(sql: &str, results: Vec<d1::RawResult>) -> Self {
        // Show the last statement that returned columns (a SELECT), else the last.
        let pick = results
            .iter()
            .rposition(|r| !r.results.columns.is_empty())
            .or(results.len().checked_sub(1));
        let r = pick.map(|i| results[i].clone());
        let columns = r
            .as_ref()
            .map(|r| r.results.columns.clone())
            .unwrap_or_default();
        let rows = r
            .as_ref()
            .map(|r| r.results.rows.clone())
            .unwrap_or_default();
        let meta = r.map(|r| r.meta).unwrap_or_default();
        let mut widths: Vec<usize> = columns.iter().map(|c| c.chars().count().max(3)).collect();
        for row in rows.iter().take(200) {
            for (i, v) in row.iter().enumerate() {
                if let Some(w) = widths.get_mut(i) {
                    let l = crate::json_view::cell_text(v).chars().count();
                    *w = (*w).max(l).min(40);
                }
            }
        }
        Self {
            columns,
            rows,
            widths,
            meta,
            sql: sql.to_string(),
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum D1View {
    #[default]
    Tables,
    Grid,
}

#[derive(Default)]
pub struct PagesState {
    pub list: ListState<pages::Project>,
    pub deployments: Vec<pages::Deployment>,
    pub deployments_for: Option<String>,
    pub dep: Cursor,
    pub loading: bool,
}

#[derive(Default)]
pub struct D1State {
    pub list: ListState<d1::Database>,
    pub tables: Vec<d1::Table>,
    pub tables_for: Option<String>,
    pub tables_loading: bool,
    pub table: Cursor,
    pub view: D1View,
    pub grid: Option<Grid>,
    pub grid_loading: bool,
    pub grid_error: Option<String>,
    pub row: Cursor,
    pub hscroll: usize,
    /// Columns of the table the grid currently shows (for row edits).
    pub columns: Vec<d1::Column>,
    pub columns_for: Option<(String, String)>,
    pub current_table: Option<String>,
    pub last_sql: String,
    pub bookmarks: HashMap<String, String>,
}

#[derive(Default)]
pub struct KvState {
    pub list: ListState<kv::Namespace>,
    pub keys: Vec<kv::Key>,
    pub keys_for: Option<String>,
    pub prefix: String,
    pub cursor_token: Option<String>,
    pub key: Cursor,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum R2Entry {
    Up,
    Folder(String),
    Object(r2::Object),
}

#[derive(Default)]
pub struct R2State {
    pub list: ListState<r2::Bucket>,
    pub info: Option<BucketInfo>,
    pub prefix: String,
    pub entries: Vec<R2Entry>,
    pub entries_for: Option<(String, String)>,
    pub cursor_token: Option<String>,
    pub entry: Cursor,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct QueuesState {
    pub list: ListState<queues::Queue>,
    pub pulled: Vec<queues::PulledMessage>,
    pub pulled_for: Option<String>,
    pub backlog: Option<u64>,
    pub msg: Cursor,
    pub scroll: usize,
}

#[derive(Default)]
pub struct DoState {
    pub list: ListState<dos::Namespace>,
    pub objects: Vec<dos::ObjectId>,
    pub objects_for: Option<String>,
    pub cursor_token: Option<String>,
    pub object: Cursor,
    pub loading: bool,
}

pub const TAIL_CAP: usize = 2000;

pub struct TailState {
    pub script: String,
    pub events: VecDeque<TailEvent>,
    pub sel: Option<usize>,
    pub paused: bool,
    pub status: String,
    pub filter: String,
    pub pending: Vec<TailEvent>,
    pub seen: u64,
}

impl TailState {
    pub fn visible(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        self.events
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                f.is_empty()
                    || e.summary.to_lowercase().contains(&f)
                    || e.logs.iter().any(|(_, m)| m.to_lowercase().contains(&f))
                    || e.exceptions.iter().any(|m| m.to_lowercase().contains(&f))
                    || e.outcome.to_lowercase().contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

pub struct Toast {
    pub text: String,
    pub at: Instant,
    pub error: bool,
}

/// Rects recorded at draw time for mouse hit-testing.
#[derive(Default, Clone, Copy)]
pub struct Rects {
    pub body: Rect,
    pub sidebar: Rect,
    pub list: Rect,
    pub detail: Rect,
    pub list_inner: Rect,
    pub detail_inner: Rect,
}

pub struct App {
    pub handle: Handle,
    pub client: Client,
    pub account: Account,
    pub accounts: Vec<Account>,
    pub cred_source: CredSource,
    pub cred_expires: Option<String>,
    pub profile: Option<String>,
    /// Sidebar width in cells and list share of the rest in percent.
    pub sidebar_w: u16,
    pub list_pct: u16,
    /// Which divider a mouse drag is moving, if any.
    pub dragging: Option<Divider>,
    pub read_only: bool,
    pub section: Section,
    pub focus: Focus,
    pub modal: Modal,
    pub workers: WorkersState,
    pub pages: PagesState,
    pub d1: D1State,
    pub kv: KvState,
    pub r2: R2State,
    pub queues: QueuesState,
    pub dos: DoState,
    pub tail: Option<TailState>,
    pub toasts: Vec<Toast>,
    pub ops_log: Vec<String>,
    pub next_req: u64,
    pub inflight: usize,
    pub filter_mode: bool,
    pub rects: Rects,
    pub should_quit: bool,
    pub tick: u64,
    pub config: crate::config::Config,
    pub last_cf: Option<String>,
    /// Set when the list selection moves; the detail loads after a short idle.
    pub select_changed: Option<Instant>,
    pub pending_download: Option<String>,
    pub pending_edit: Option<(String, String)>,
    pub pending_meta_edit: Option<(String, String)>,
}

impl App {
    pub fn new(
        handle: Handle,
        client: Client,
        account: Account,
        accounts: Vec<Account>,
        cred: &Credential,
        read_only: bool,
        config: crate::config::Config,
    ) -> Self {
        Self {
            handle,
            client,
            account,
            accounts,
            cred_source: cred.source,
            cred_expires: cred.expires.clone(),
            profile: None,
            sidebar_w: config.sidebar_width.unwrap_or(18).clamp(12, 40),
            list_pct: config.list_percent.unwrap_or(33).clamp(15, 85),
            dragging: None,
            read_only,
            section: Section::Workers,
            focus: Focus::List,
            modal: Modal::None,
            workers: WorkersState::default(),
            pages: PagesState::default(),
            d1: D1State::default(),
            kv: KvState::default(),
            r2: R2State::default(),
            queues: QueuesState::default(),
            dos: DoState::default(),
            tail: None,
            toasts: Vec::new(),
            ops_log: Vec::new(),
            next_req: 1,
            inflight: 0,
            filter_mode: false,
            rects: Rects::default(),
            should_quit: false,
            tick: 0,
            config,
            last_cf: None,
            select_changed: None,
            pending_download: None,
            pending_edit: None,
            pending_meta_edit: None,
        }
    }

    pub fn send(&mut self, cmd: Command) -> u64 {
        let id = self.next_req;
        self.next_req += 1;
        // Tail and account commands never answer with a Done event.
        let counted = !matches!(
            cmd,
            Command::StartTail { .. } | Command::StopTail | Command::SetAccount(_)
        );
        if counted {
            self.inflight += 1;
        }
        if self.handle.tx.try_send((id, cmd)).is_err() {
            if counted {
                self.inflight = self.inflight.saturating_sub(1);
            }
            self.toast_err("I/O queue is full; try again".into());
        }
        id
    }

    pub fn toast_err(&mut self, text: String) {
        self.ops_log.push(format!("{} ERROR {text}", clock_utc()));
        self.toasts.push(Toast {
            text,
            at: Instant::now(),
            error: true,
        });
    }

    pub fn toast_info(&mut self, text: String) {
        self.toasts.push(Toast {
            text,
            at: Instant::now(),
            error: false,
        });
    }

    pub fn log(&mut self, text: String) {
        self.ops_log.push(format!("{} {text}", clock_utc()));
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let now = Instant::now();
        if let Some(t) = self.select_changed
            && now.duration_since(t).as_millis() >= 200
        {
            self.select_changed = None;
            self.load_selection();
        }
        self.toasts
            .retain(|t| now.duration_since(t.at).as_secs_f32() < if t.error { 6.0 } else { 3.5 });
        if let Some(t) = &mut self.tail
            && !t.paused
            && !t.pending.is_empty()
        {
            for e in t.pending.drain(..) {
                t.events.push_back(e);
            }
            while t.events.len() > TAIL_CAP {
                t.events.pop_front();
                if let Some(s) = &mut t.sel {
                    *s = s.saturating_sub(1);
                }
            }
        }
    }

    /// Load the current section's list if it has not been loaded yet.
    pub fn ensure_loaded(&mut self) {
        match self.section {
            Section::Workers if !self.workers.list.loaded && !self.workers.list.loading => {
                self.workers.list.loading = true;
                self.send(Command::ListScripts);
            }
            Section::Pages if !self.pages.list.loaded && !self.pages.list.loading => {
                self.pages.list.loading = true;
                self.send(Command::ListPagesProjects);
            }
            Section::D1 if !self.d1.list.loaded && !self.d1.list.loading => {
                self.d1.list.loading = true;
                self.send(Command::ListDatabases);
            }
            Section::Kv if !self.kv.list.loaded && !self.kv.list.loading => {
                self.kv.list.loading = true;
                self.send(Command::ListNamespaces);
            }
            Section::R2 if !self.r2.list.loaded && !self.r2.list.loading => {
                self.r2.list.loading = true;
                self.send(Command::ListBuckets);
            }
            Section::Queues if !self.queues.list.loaded && !self.queues.list.loading => {
                self.queues.list.loading = true;
                self.send(Command::ListQueues);
            }
            Section::DurableObjects if !self.dos.list.loaded && !self.dos.list.loading => {
                self.dos.list.loading = true;
                self.send(Command::ListDoNamespaces);
            }
            _ => {}
        }
    }

    pub fn refresh(&mut self) {
        match self.section {
            Section::Workers => {
                self.workers.list.loading = true;
                self.send(Command::ListScripts);
                self.workers.detail = None;
                self.load_selection();
            }
            Section::Pages => {
                self.pages.list.loading = true;
                self.send(Command::ListPagesProjects);
                self.pages.deployments_for = None;
                self.load_selection();
            }
            Section::D1 => {
                self.d1.list.loading = true;
                self.send(Command::ListDatabases);
                self.d1.tables_for = None;
                if self.d1.view == D1View::Grid
                    && !self.d1.last_sql.is_empty()
                    && let Some(db) = self.d1.list.current().map(|d| d.uuid.clone())
                {
                    self.run_sql(db, self.d1.last_sql.clone());
                }
                self.load_selection();
            }
            Section::Kv => {
                self.kv.list.loading = true;
                self.send(Command::ListNamespaces);
                self.kv.keys_for = None;
                self.load_selection();
            }
            Section::R2 => {
                self.r2.list.loading = true;
                self.send(Command::ListBuckets);
                self.r2.entries_for = None;
                self.load_selection();
            }
            Section::Queues => {
                self.queues.list.loading = true;
                self.send(Command::ListQueues);
            }
            Section::DurableObjects => {
                self.dos.list.loading = true;
                self.send(Command::ListDoNamespaces);
                self.dos.objects_for = None;
                self.load_selection();
            }
        }
    }

    /// Fetch the detail for whatever the list has selected, if not cached.
    pub fn load_selection(&mut self) {
        match self.section {
            Section::Workers => {
                let Some(name) = self.workers.list.current().map(|s| s.id.clone()) else {
                    return;
                };
                if self.workers.detail_for != name || self.workers.detail.is_none() {
                    if self.workers.detail_for != name {
                        self.workers.detail = None;
                        self.workers.scroll = 0;
                        self.workers.versions = Cursor::default();
                        self.workers.versions_mode = false;
                    }
                    self.workers.detail_for = name.clone();
                    self.workers.detail_loading = true;
                    self.send(Command::LoadScript { name });
                }
            }
            Section::Pages => {
                let Some(name) = self.pages.list.current().map(|p| p.name.clone()) else {
                    return;
                };
                if self.pages.deployments_for.as_deref() != Some(&name) {
                    self.pages.deployments.clear();
                    self.pages.dep = Cursor::default();
                    self.pages.deployments_for = Some(name.clone());
                    self.pages.loading = true;
                    self.send(Command::LoadPagesDeployments { project: name });
                }
            }
            Section::D1 => {
                let Some(db) = self.d1.list.current().map(|d| d.uuid.clone()) else {
                    return;
                };
                if self.d1.tables_for.as_deref() != Some(&db) {
                    self.d1.tables.clear();
                    self.d1.table = Cursor::default();
                    self.d1.view = D1View::Tables;
                    self.d1.grid = None;
                    self.d1.current_table = None;
                    self.d1.tables_for = Some(db.clone());
                    self.d1.tables_loading = true;
                    self.send(Command::GetDatabase { id: db.clone() });
                    self.send(Command::LoadTables { db });
                }
            }
            Section::Kv => {
                let Some(ns) = self.kv.list.current().map(|n| n.id.clone()) else {
                    return;
                };
                if self.kv.keys_for.as_deref() != Some(&ns) {
                    self.kv.keys.clear();
                    self.kv.key = Cursor::default();
                    self.kv.cursor_token = None;
                    self.kv.prefix.clear();
                    self.kv.keys_for = Some(ns.clone());
                    self.kv.loading = true;
                    self.kv.error = None;
                    self.send(Command::ListKeys {
                        ns,
                        prefix: String::new(),
                        cursor: None,
                    });
                }
            }
            Section::R2 => {
                let Some(bucket) = self.r2.list.current().map(|b| b.name.clone()) else {
                    return;
                };
                if self.r2.entries_for.as_ref().map(|(b, _)| b.as_str()) != Some(&bucket) {
                    self.r2.prefix.clear();
                    self.r2.info = None;
                    self.load_objects(bucket.clone(), String::new());
                    self.send(Command::LoadBucketInfo { name: bucket });
                }
            }
            Section::Queues => {
                let Some(q) = self.queues.list.current() else {
                    return;
                };
                if self.queues.pulled_for.as_deref() != Some(&q.queue_id) {
                    self.queues.pulled.clear();
                    self.queues.pulled_for = Some(q.queue_id.clone());
                    self.queues.msg = Cursor::default();
                    self.queues.scroll = 0;
                }
            }
            Section::DurableObjects => {
                let Some(ns) = self.dos.list.current().map(|n| n.id.clone()) else {
                    return;
                };
                if self.dos.objects_for.as_deref() != Some(&ns) {
                    self.dos.objects.clear();
                    self.dos.object = Cursor::default();
                    self.dos.cursor_token = None;
                    self.dos.objects_for = Some(ns.clone());
                    self.dos.loading = true;
                    self.send(Command::ListDoObjects { ns, cursor: None });
                }
            }
        }
    }

    pub fn load_objects(&mut self, bucket: String, prefix: String) {
        self.r2.entries.clear();
        self.r2.entry = Cursor::default();
        self.r2.cursor_token = None;
        self.r2.entries_for = Some((bucket.clone(), prefix.clone()));
        self.r2.prefix = prefix.clone();
        self.r2.loading = true;
        self.r2.error = None;
        self.send(Command::ListObjects {
            bucket,
            prefix,
            cursor: None,
        });
    }

    pub fn load_keys(&mut self, ns: String, prefix: String) {
        self.kv.keys.clear();
        self.kv.key = Cursor::default();
        self.kv.cursor_token = None;
        self.kv.keys_for = Some(ns.clone());
        self.kv.prefix = prefix.clone();
        self.kv.loading = true;
        self.kv.error = None;
        self.send(Command::ListKeys {
            ns,
            prefix,
            cursor: None,
        });
    }

    pub fn run_sql(&mut self, db: String, sql: String) {
        self.d1.grid_loading = true;
        self.d1.grid_error = None;
        self.d1.last_sql = sql.clone();
        self.d1.view = D1View::Grid;
        self.send(Command::RunSql { db, sql });
    }

    pub fn open_table(&mut self, db: String, table: String) {
        self.d1.current_table = Some(table.clone());
        self.d1.row = Cursor::default();
        self.d1.hscroll = 0;
        if self.d1.columns_for.as_ref() != Some(&(db.clone(), table.clone())) {
            self.d1.columns.clear();
            self.d1.columns_for = Some((db.clone(), table.clone()));
            self.send(Command::LoadColumns {
                db: db.clone(),
                table: table.clone(),
            });
        }
        let sql = format!("SELECT * FROM {} LIMIT 200;", d1::quote_ident(&table));
        self.run_sql(db, sql);
    }

    pub fn on_core(&mut self, ev: CoreEvent) {
        match ev {
            CoreEvent::Done { req_id: _, result } => {
                self.inflight = self.inflight.saturating_sub(1);
                match result {
                    Ok(p) => self.on_payload(p),
                    Err(e) => self.on_api_error(e),
                }
            }
            CoreEvent::Tail(e) => {
                if let Some(t) = &mut self.tail {
                    t.seen += 1;
                    if t.paused {
                        t.pending.push(e);
                        if t.pending.len() > TAIL_CAP {
                            t.pending.remove(0);
                        }
                    } else {
                        t.events.push_back(e);
                        while t.events.len() > TAIL_CAP {
                            t.events.pop_front();
                            if let Some(s) = &mut t.sel {
                                *s = s.saturating_sub(1);
                            }
                        }
                    }
                }
            }
            CoreEvent::TailInfo(s) => {
                if let Some(t) = &mut self.tail {
                    t.status = s;
                }
            }
            CoreEvent::TailClosed(s) => {
                if let Some(t) = &mut self.tail {
                    t.status = format!("closed: {s}");
                }
                self.toast_err(format!("tail: {s}"));
            }
        }
    }

    fn on_api_error(&mut self, e: ApiError) {
        // Clear loading flags so the UI does not spin forever, and pin the
        // message on whichever list was waiting so an empty pane explains itself.
        let msg_for_list = match &e {
            ApiError::MissingPermission { .. } => {
                "token lacks permission for this product".to_string()
            }
            other => other.to_string(),
        };
        if self.workers.list.loading {
            self.workers.list.error = Some(msg_for_list.clone());
        }
        if self.pages.list.loading {
            self.pages.list.error = Some(msg_for_list.clone());
        }
        if self.d1.list.loading {
            self.d1.list.error = Some(msg_for_list.clone());
        }
        if self.kv.list.loading {
            self.kv.list.error = Some(msg_for_list.clone());
        }
        if self.r2.list.loading {
            self.r2.list.error = Some(msg_for_list.clone());
        }
        if self.queues.list.loading {
            self.queues.list.error = Some(msg_for_list.clone());
        }
        if self.dos.list.loading {
            self.dos.list.error = Some(msg_for_list);
        }
        self.workers.list.loading = false;
        self.workers.detail_loading = false;
        self.pages.list.loading = false;
        self.pages.loading = false;
        self.d1.list.loading = false;
        self.d1.tables_loading = false;
        if self.d1.grid_loading {
            self.d1.grid_loading = false;
            self.d1.grid_error = Some(e.to_string());
        }
        self.kv.list.loading = false;
        if self.kv.loading {
            self.kv.loading = false;
            self.kv.error = Some(e.to_string());
        }
        self.r2.list.loading = false;
        if self.r2.loading {
            self.r2.loading = false;
            self.r2.error = Some(e.to_string());
        }
        self.queues.list.loading = false;
        self.dos.list.loading = false;
        self.dos.loading = false;
        let msg = match &e {
            ApiError::MissingPermission { .. } => {
                "your token lacks permission for this resource".to_string()
            }
            ApiError::RateLimited { retry_after } => {
                format!("API rate limit hit; wait {retry_after}s")
            }
            other => other.to_string(),
        };
        self.toast_err(msg);
    }

    fn on_payload(&mut self, p: Payload) {
        match p {
            Payload::Verified(_) => {}
            Payload::Accounts(list) => {
                if let Some(a) = list.iter().find(|a| a.id == self.account.id) {
                    self.account = a.clone();
                } else if let Some(a) = list.first() {
                    self.account = a.clone();
                    self.send(Command::SetAccount(a.id.clone()));
                    self.ensure_loaded();
                }
                self.accounts = list;
            }
            Payload::Scripts(v) => {
                let keep = self.workers.list.current().map(|s| s.id.clone());
                self.workers.list.set(v, |s| s.id.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .workers
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.workers.list.items[i].id == k)
                {
                    self.workers.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::Script(d) => {
                if d.name == self.workers.detail_for {
                    for e in &d.errors {
                        self.log(format!("worker {}: {e}", d.name));
                    }
                    self.workers.detail_loading = false;
                    self.workers.detail = Some(d);
                }
            }
            Payload::ScriptContent { name, parts } => {
                self.open_code_viewer(name, parts);
            }
            Payload::PagesProjects(v) => {
                let keep = self.pages.list.current().map(|p| p.name.clone());
                self.pages.list.set(v, |p| p.name.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .pages
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.pages.list.items[i].name == k)
                {
                    self.pages.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::PagesDeployments { project, items } => {
                if self.pages.deployments_for.as_deref() == Some(&project) {
                    self.pages.deployments = items;
                    self.pages.loading = false;
                    self.pages.dep.clamp(self.pages.deployments.len());
                }
            }
            Payload::PagesLogs { project, id, lines } => {
                let text = lines.join("\n");
                let l = crate::json_view::text_lines(&text);
                self.open_viewer(
                    &format!("{project} · build log {}", &id[..8.min(id.len())]),
                    l,
                    text,
                );
            }
            Payload::Databases(v) => {
                let keep = self.d1.list.current().map(|d| d.uuid.clone());
                self.d1.list.set(v, |d| d.name.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .d1
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.d1.list.items[i].uuid == k)
                {
                    self.d1.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::Database(d) => {
                if let Some(item) = self.d1.list.items.iter_mut().find(|x| x.uuid == d.uuid) {
                    *item = d;
                }
            }
            Payload::Tables { db, tables } => {
                if self.d1.tables_for.as_deref() == Some(&db) {
                    self.d1.tables = tables;
                    self.d1.tables_loading = false;
                    self.d1.table.clamp(self.d1.tables.len());
                }
            }
            Payload::Columns { db, table, columns } => {
                if self.d1.columns_for.as_ref() == Some(&(db, table)) {
                    self.d1.columns = columns;
                }
            }
            Payload::SqlResult { db, sql, results } => {
                if self.d1.list.current().map(|d| d.uuid.as_str()) == Some(&db) {
                    let had_write = results.iter().any(|r| {
                        r.meta.rows_written.unwrap_or(0) > 0 || r.meta.changes.unwrap_or(0) > 0
                    });
                    let grid = Grid::from_results(&sql, results);
                    self.d1.grid_loading = false;
                    self.d1.grid_error = None;
                    self.d1.row.clamp(grid.rows.len());
                    self.d1.grid = Some(grid);
                    self.d1.view = D1View::Grid;
                    if had_write {
                        self.log(format!("d1 write: {}", crate::util::one_line(&sql, 120)));
                        // Schema may have changed: refresh the table list in place.
                        self.d1.tables_loading = true;
                        self.send(Command::LoadTables { db: db.clone() });
                        // A write through the row editor: show the table again.
                        if let Some(t) = self.d1.current_table.clone() {
                            let select =
                                format!("SELECT * FROM {} LIMIT 200;", d1::quote_ident(&t));
                            if sql != select {
                                self.run_sql(db, select);
                            }
                        }
                    }
                }
            }
            Payload::Bookmark { id, bookmark } => {
                self.toast_info(format!("bookmark {bookmark}"));
                self.d1.bookmarks.insert(id, bookmark);
            }
            Payload::Namespaces(v) => {
                let keep = self.kv.list.current().map(|n| n.id.clone());
                self.kv.list.set(v, |n| n.title.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .kv
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.kv.list.items[i].id == k)
                {
                    self.kv.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::Keys {
                ns,
                prefix,
                page,
                cursor,
                append,
            } => {
                if self.kv.keys_for.as_deref() == Some(&ns) && self.kv.prefix == prefix {
                    if append {
                        self.kv.keys.extend(page);
                    } else {
                        self.kv.keys = page;
                    }
                    self.kv.cursor_token = cursor;
                    self.kv.loading = false;
                    self.kv.key.clamp(self.kv.keys.len());
                }
            }
            Payload::Value {
                ns,
                key,
                bytes,
                expiration,
                metadata,
            } => {
                if self.pending_edit.take() == Some((ns.clone(), key.clone())) {
                    let (text, is_text) = crate::util::bytes_to_text(&bytes);
                    if !is_text {
                        self.toast_err("value is binary; editing is only for text values".into());
                        return;
                    }
                    self.open_editor(
                        &format!("edit {key}"),
                        "Ctrl-S saves (then asks for TTL) · Esc cancels",
                        &text,
                        crate::modal::EditorAction::KvPut { ns, key, metadata },
                    );
                } else if self.pending_meta_edit.take() == Some((ns.clone(), key.clone())) {
                    let text = metadata
                        .as_ref()
                        .map(|m| serde_json::to_string_pretty(m).unwrap_or_default())
                        .unwrap_or_else(|| "{}".into());
                    self.open_editor(
                        &format!("metadata of {key}"),
                        "JSON ≤ 1024 bytes · Ctrl-S saves (re-puts the value with this metadata) · empty removes it",
                        &text,
                        crate::modal::EditorAction::KvMetadata { ns, key, value: bytes },
                    );
                } else {
                    self.open_value_viewer(key, bytes, expiration, metadata);
                }
            }
            Payload::Buckets(v) => {
                let keep = self.r2.list.current().map(|b| b.name.clone());
                self.r2.list.set(v, |b| b.name.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .r2
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.r2.list.items[i].name == k)
                {
                    self.r2.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::Objects {
                bucket,
                prefix,
                objects,
                folders,
                cursor,
                append,
            } => {
                if self.r2.entries_for == Some((bucket, prefix.clone())) {
                    if !append {
                        self.r2.entries.clear();
                        if !prefix.is_empty() {
                            self.r2.entries.push(R2Entry::Up);
                        }
                        for f in folders {
                            self.r2.entries.push(R2Entry::Folder(f));
                        }
                    } else {
                        for f in folders {
                            if !self
                                .r2
                                .entries
                                .iter()
                                .any(|e| matches!(e, R2Entry::Folder(x) if *x == f))
                            {
                                self.r2.entries.push(R2Entry::Folder(f));
                            }
                        }
                    }
                    for o in objects {
                        // the prefix "folder marker" object itself is noise
                        if o.key == prefix {
                            continue;
                        }
                        self.r2.entries.push(R2Entry::Object(o));
                    }
                    self.r2.cursor_token = cursor;
                    self.r2.loading = false;
                    self.r2.entry.clamp(self.r2.entries.len());
                }
            }
            Payload::Object {
                bucket: _,
                key,
                bytes,
                content_type,
            } => {
                if !self.finish_download(&key, &bytes) {
                    self.open_object_viewer(key, bytes, content_type);
                }
            }
            Payload::BucketInfo(info) => {
                if let Some(item) = self
                    .r2
                    .list
                    .items
                    .iter_mut()
                    .find(|b| b.name == info.bucket.name)
                {
                    *item = info.bucket.clone();
                }
                if self.r2.list.current().map(|b| b.name.as_str())
                    == Some(info.bucket.name.as_str())
                {
                    self.r2.info = Some(info);
                }
            }
            Payload::Queues(v) => {
                let keep = self.queues.list.current().map(|q| q.queue_id.clone());
                self.queues.list.set(v, |q| q.queue_name.clone());
                if let Some(k) = keep
                    && let Some(pos) = self
                        .queues
                        .list
                        .visible
                        .iter()
                        .position(|&i| self.queues.list.items[i].queue_id == k)
                {
                    self.queues.list.selected = pos;
                }
                self.load_selection();
            }
            Payload::Pulled { id, result } => {
                if self.queues.pulled_for.as_deref() == Some(&id) {
                    self.queues.backlog = Some(result.message_backlog_count);
                    let n = result.messages.len();
                    self.queues.pulled.extend(result.messages);
                    self.queues.msg.clamp(self.queues.pulled.len());
                    self.toast_info(format!("pulled {n} messages (leased 30s)"));
                }
            }
            Payload::DoNamespaces(v) => {
                self.dos.list.set(v, |n| {
                    format!(
                        "{} {}",
                        n.script.clone().unwrap_or_default(),
                        n.class.clone().unwrap_or_default()
                    )
                });
                self.load_selection();
            }
            Payload::DoObjects {
                ns,
                objects,
                cursor,
                append,
            } => {
                if self.dos.objects_for.as_deref() == Some(&ns) {
                    if append {
                        self.dos.objects.extend(objects);
                    } else {
                        self.dos.objects = objects;
                    }
                    self.dos.cursor_token = cursor;
                    self.dos.loading = false;
                    self.dos.object.clamp(self.dos.objects.len());
                }
            }
            Payload::Written { what } => {
                self.log(what.clone());
                self.toast_info(what);
                self.after_write();
            }
        }
    }

    /// Reload what a write could have changed.
    fn after_write(&mut self) {
        match self.section {
            Section::Workers => {
                let name = self.workers.detail_for.clone();
                if !name.is_empty() {
                    self.workers.detail_loading = true;
                    self.send(Command::LoadScript { name });
                }
                self.workers.list.loading = true;
                self.send(Command::ListScripts);
            }
            Section::Pages => {
                self.pages.list.loading = true;
                self.send(Command::ListPagesProjects);
                self.pages.deployments_for = None;
            }
            Section::D1 => {
                self.d1.list.loading = true;
                self.send(Command::ListDatabases);
            }
            Section::Kv => {
                if let Some(ns) = self.kv.keys_for.clone() {
                    let prefix = self.kv.prefix.clone();
                    self.load_keys(ns, prefix);
                }
                self.kv.list.loading = true;
                self.send(Command::ListNamespaces);
            }
            Section::R2 => {
                if let Some((b, p)) = self.r2.entries_for.clone() {
                    self.load_objects(b.clone(), p);
                    self.send(Command::LoadBucketInfo { name: b });
                }
                self.r2.list.loading = true;
                self.send(Command::ListBuckets);
            }
            Section::Queues => {
                self.queues.list.loading = true;
                self.send(Command::ListQueues);
            }
            Section::DurableObjects => {}
        }
    }

    pub fn resize_sidebar(&mut self, delta: i16) {
        self.sidebar_w = (self.sidebar_w as i16 + delta).clamp(12, 40) as u16;
        self.save_sizes();
    }

    pub fn resize_list(&mut self, delta: i16) {
        self.list_pct = (self.list_pct as i16 + delta).clamp(15, 85) as u16;
        self.save_sizes();
    }

    pub fn reset_sizes(&mut self) {
        self.sidebar_w = 18;
        self.list_pct = 33;
        self.save_sizes();
    }

    fn save_sizes(&mut self) {
        self.config.sidebar_width = Some(self.sidebar_w);
        self.config.list_percent = Some(self.list_pct);
        let _ = crate::config::save(&self.config);
    }

    pub fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];
        FRAMES[(self.tick % 4) as usize]
    }

    pub fn account_label(&self) -> String {
        let id = &self.account.id;
        format!("{} ({}…)", self.account.name, &id[..id.len().min(6)])
    }
}
