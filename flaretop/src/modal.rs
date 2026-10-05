//! Overlays that capture input while open.

use flaretop_core::auth::Account;
use flaretop_core::products::d1::Column;
use ratatui::text::Line;
use serde_json::Value;

use crate::input::Input;
use crate::textarea::TextArea;

pub enum Modal {
    None,
    Help,
    Confirm(Confirm),
    Prompt(Prompt),
    Editor(Editor),
    Viewer(Viewer),
    Palette(Palette),
    Accounts {
        items: Vec<Account>,
        selected: usize,
    },
    Profiles {
        items: Vec<(String, flaretop_core::auth::Profile)>,
        selected: usize,
    },
    /// First-run token entry when nothing was discovered.
    OpsLog {
        scroll: usize,
    },
}

impl Modal {
    pub fn is_open(&self) -> bool {
        !matches!(self, Modal::None)
    }
}

/// What to do once a confirm box is accepted.
#[derive(Debug, Clone)]
pub enum PendingAction {
    DeleteScript {
        name: String,
    },
    PagesRollback {
        project: String,
        id: String,
    },
    PagesRetry {
        project: String,
        id: String,
    },
    DeletePagesProject {
        name: String,
    },
    DeleteSecret {
        name: String,
        key: String,
    },
    SetSubdomain {
        name: String,
        enabled: bool,
    },
    Deploy {
        name: String,
        versions: Vec<(String, f64)>,
        label: String,
    },
    DeleteDatabase {
        id: String,
        name: String,
    },
    SqlWrite {
        db: String,
        sql: String,
    },
    DeleteKey {
        ns: String,
        key: String,
    },
    BulkDeleteKeys {
        ns: String,
        keys: Vec<String>,
    },
    DeleteNamespace {
        id: String,
        title: String,
    },
    DeleteObject {
        bucket: String,
        key: String,
    },
    DeleteBucket {
        name: String,
    },
    SetManagedDomain {
        bucket: String,
        enabled: bool,
    },
    PurgeQueue {
        id: String,
        name: String,
    },
    DeleteQueue {
        id: String,
        name: String,
    },
    SetQueuePaused {
        id: String,
        paused: bool,
    },
    AckMessages {
        id: String,
        acks: Vec<String>,
        retries: Vec<String>,
    },
}

pub struct Confirm {
    pub title: String,
    pub body: Vec<String>,
    /// When set, the user must type exactly this to enable Enter.
    pub must_type: Option<String>,
    pub input: Input,
    pub action: PendingAction,
    pub danger: bool,
}

impl Confirm {
    pub fn typed_ok(&self) -> bool {
        match &self.must_type {
            None => true,
            Some(expected) => self.input.text.trim() == expected,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PromptAction {
    SecretName {
        script: String,
    },
    SecretValue {
        script: String,
        key: String,
    },
    Cron {
        script: String,
    },
    GradualPercent {
        script: String,
        version: String,
        current: Option<String>,
    },
    NewDatabase,
    NewNamespace,
    NewKey {
        ns: String,
    },
    KeyPrefix {
        ns: String,
    },
    KeyTtl {
        ns: String,
        key: String,
        value: Vec<u8>,
        metadata: Option<Value>,
    },
    NewBucket,
    DownloadPath {
        bucket: String,
        key: String,
    },
    UploadPath {
        bucket: String,
        prefix: String,
    },
    NewQueue,
    PullBatch {
        id: String,
    },
    SaveToken,
    TailFilter,
}

pub struct Prompt {
    pub title: String,
    pub label: String,
    pub input: Input,
    pub secret: bool,
    pub action: PromptAction,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum EditorAction {
    Sql {
        db: String,
    },
    KvPut {
        ns: String,
        key: String,
        metadata: Option<Value>,
    },
    KvMetadata {
        ns: String,
        key: String,
        value: Vec<u8>,
    },
    PushMessage {
        id: String,
    },
    D1EditRow {
        db: String,
        table: String,
        cols: Vec<Column>,
        old: Vec<Value>,
    },
    D1InsertRow {
        db: String,
        table: String,
        cols: Vec<Column>,
    },
}

pub struct Editor {
    pub title: String,
    pub hint: String,
    pub area: TextArea,
    pub action: EditorAction,
    pub error: Option<String>,
}

/// Read-only scrollable text (values, code, raw events, help).
pub struct Viewer {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub scroll: usize,
    pub hscroll: usize,
    /// Text to put on the clipboard with `y`.
    pub copy_text: String,
    /// Optional sections (multi-part script bundles): (name, first line).
    pub sections: Vec<(String, usize)>,
}

impl Viewer {
    pub fn new(title: impl Into<String>, lines: Vec<Line<'static>>, copy_text: String) -> Self {
        Self {
            title: title.into(),
            lines,
            scroll: 0,
            hscroll: 0,
            copy_text,
            sections: Vec::new(),
        }
    }
}

/// Every UI action reachable from the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    GoSection(usize),
    Refresh,
    Help,
    OpsLog,
    Accounts,
    Profiles,
    ToggleReadOnly,
    Quit,
    SetTheme(&'static str),
    SaveToken,
    ForgetToken,
    CopyCf,
    /// Section-specific actions are dispatched by key name.
    Key(char),
}

pub struct Palette {
    pub title: String,
    pub input: Input,
    pub actions: Vec<(String, AppAction)>,
    pub filtered: Vec<usize>,
    pub selected: usize,
}

impl Palette {
    pub fn new(title: impl Into<String>, actions: Vec<(String, AppAction)>) -> Self {
        let filtered = (0..actions.len()).collect();
        Self {
            title: title.into(),
            input: Input::default(),
            actions,
            filtered,
            selected: 0,
        }
    }

    pub fn refilter(&mut self) {
        let needle = self.input.text.trim().to_lowercase();
        let mut scored: Vec<(i32, usize)> = self
            .actions
            .iter()
            .enumerate()
            .filter_map(|(i, (label, _))| fuzzy_score(&needle, label).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    pub fn selected_action(&self) -> Option<AppAction> {
        self.filtered
            .get(self.selected)
            .map(|&i| self.actions[i].1.clone())
    }
}

/// Case-insensitive subsequence match; higher scores for consecutive runs
/// and earlier matches. None = no match.
pub fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut score = 0i32;
    let mut hi = 0usize;
    let mut last_hit: Option<usize> = None;
    for nc in needle.chars() {
        if nc == ' ' {
            last_hit = None;
            continue;
        }
        let mut found = None;
        while hi < hay.len() {
            if hay[hi] == nc {
                found = Some(hi);
                break;
            }
            hi += 1;
        }
        let pos = found?;
        score += match last_hit {
            Some(prev) if pos == prev + 1 => 3,
            _ => 1,
        };
        last_hit = Some(pos);
        hi = pos + 1;
    }
    score -= (last_hit.unwrap_or(0) as i32) / 8;
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_basics() {
        assert!(fuzzy_score("", "anything").is_some());
        assert!(fuzzy_score("tbl", "view: toggle json/table (v)").is_some());
        assert!(fuzzy_score("zzz", "view: toggle").is_none());
        let a = fuzzy_score("theme", "theme: dark").unwrap();
        let b = fuzzy_score("theme", "t h e m e scattered").unwrap();
        assert!(a > b);
    }
}
