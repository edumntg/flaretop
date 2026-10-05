//! Commands the UI sends to the actor and the events it gets back.

use serde_json::Value;

use crate::api::ApiError;
use crate::auth::{Account, TokenStatus};
use crate::products::{d1, durable_objects as dos, kv, pages, queues, r2, workers};
use crate::tail::{TailEvent, TailFilters};

/// Every command carries a `req_id` chosen by the UI; the matching event
/// echoes it so stale responses can be dropped.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Verify,
    ListAccounts,
    /// Switch the account every later command targets.
    SetAccount(String),

    ListScripts,
    LoadScript {
        name: String,
    },
    LoadScriptContent {
        name: String,
    },
    PutSecret {
        name: String,
        key: String,
        value: String,
    },
    DeleteSecret {
        name: String,
        key: String,
    },
    PutSchedules {
        name: String,
        crons: Vec<String>,
    },
    SetSubdomain {
        name: String,
        enabled: bool,
        previews: bool,
    },
    Deploy {
        name: String,
        versions: Vec<(String, f64)>,
        message: Option<String>,
    },
    DeleteScript {
        name: String,
    },

    ListPagesProjects,
    LoadPagesDeployments {
        project: String,
    },
    PagesRollback {
        project: String,
        id: String,
    },
    PagesRetry {
        project: String,
        id: String,
    },
    PagesLogs {
        project: String,
        id: String,
    },
    DeletePagesProject {
        name: String,
    },

    ListDatabases,
    GetDatabase {
        id: String,
    },
    LoadTables {
        db: String,
    },
    LoadColumns {
        db: String,
        table: String,
    },
    RunSql {
        db: String,
        sql: String,
    },
    CreateDatabase {
        name: String,
    },
    DeleteDatabase {
        id: String,
        name: String,
    },
    D1Bookmark {
        id: String,
    },

    ListNamespaces,
    ListKeys {
        ns: String,
        prefix: String,
        cursor: Option<String>,
    },
    GetValue {
        ns: String,
        key: String,
    },
    PutValue {
        ns: String,
        key: String,
        value: Vec<u8>,
        metadata: Option<Value>,
        ttl: Option<u64>,
    },
    DeleteKey {
        ns: String,
        key: String,
    },
    BulkDeleteKeys {
        ns: String,
        keys: Vec<String>,
    },
    CreateNamespace {
        title: String,
    },
    DeleteNamespace {
        id: String,
        title: String,
    },

    ListBuckets,
    ListObjects {
        bucket: String,
        prefix: String,
        cursor: Option<String>,
    },
    GetObject {
        bucket: String,
        key: String,
    },
    PutObject {
        bucket: String,
        key: String,
        bytes: Vec<u8>,
        content_type: String,
    },
    DeleteObject {
        bucket: String,
        key: String,
    },
    DeleteObjects {
        bucket: String,
        keys: Vec<String>,
    },
    CreateBucket {
        name: String,
    },
    DeleteBucket {
        name: String,
    },
    LoadBucketInfo {
        name: String,
    },
    SetManagedDomain {
        bucket: String,
        enabled: bool,
    },

    ListQueues,
    PushMessage {
        id: String,
        body: String,
        json: bool,
    },
    PurgeQueue {
        id: String,
        name: String,
    },
    SetQueuePaused {
        id: String,
        paused: bool,
    },
    CreateQueue {
        name: String,
    },
    DeleteQueue {
        id: String,
        name: String,
    },
    PullMessages {
        id: String,
        batch: usize,
    },
    AckMessages {
        id: String,
        acks: Vec<String>,
        retries: Vec<String>,
    },

    ListDoNamespaces,
    ListDoObjects {
        ns: String,
        cursor: Option<String>,
    },

    StartTail {
        script: String,
        filters: TailFilters,
    },
    StopTail,
}

/// Aggregated detail for one Worker; each part may be missing when the token
/// lacks a permission or the call failed.
#[derive(Debug, Clone, Default)]
pub struct ScriptDetail {
    pub name: String,
    pub settings: Option<Value>,
    pub schedules: Vec<workers::Schedule>,
    pub secrets: Vec<workers::Secret>,
    pub subdomain: Option<workers::Subdomain>,
    pub versions: Vec<workers::Version>,
    pub deployments: Vec<workers::Deployment>,
    pub domains: Vec<workers::Domain>,
    pub stats: Option<workers::InvocationStats>,
    pub account_subdomain: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BucketInfo {
    pub bucket: r2::Bucket,
    pub managed: Option<r2::ManagedDomain>,
    pub custom: Vec<r2::CustomDomain>,
    pub stats: Option<r2::BucketStats>,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Payload {
    Verified(TokenStatus),
    Accounts(Vec<Account>),

    Scripts(Vec<workers::Script>),
    Script(ScriptDetail),
    ScriptContent {
        name: String,
        parts: Vec<(String, String)>,
    },

    PagesProjects(Vec<pages::Project>),
    PagesDeployments {
        project: String,
        items: Vec<pages::Deployment>,
    },
    PagesLogs {
        project: String,
        id: String,
        lines: Vec<String>,
    },

    Databases(Vec<d1::Database>),
    Database(d1::Database),
    Tables {
        db: String,
        tables: Vec<d1::Table>,
    },
    Columns {
        db: String,
        table: String,
        columns: Vec<d1::Column>,
    },
    SqlResult {
        db: String,
        sql: String,
        results: Vec<d1::RawResult>,
    },
    Bookmark {
        id: String,
        bookmark: String,
    },

    Namespaces(Vec<kv::Namespace>),
    Keys {
        ns: String,
        prefix: String,
        page: Vec<kv::Key>,
        cursor: Option<String>,
        append: bool,
    },
    Value {
        ns: String,
        key: String,
        bytes: Vec<u8>,
        expiration: Option<u64>,
        metadata: Option<Value>,
    },

    Buckets(Vec<r2::Bucket>),
    Objects {
        bucket: String,
        prefix: String,
        objects: Vec<r2::Object>,
        folders: Vec<String>,
        cursor: Option<String>,
        append: bool,
    },
    Object {
        bucket: String,
        key: String,
        bytes: Vec<u8>,
        content_type: Option<String>,
    },
    BucketInfo(BucketInfo),

    Queues(Vec<queues::Queue>),
    Pulled {
        id: String,
        result: queues::PullResult,
    },

    DoNamespaces(Vec<dos::Namespace>),
    DoObjects {
        ns: String,
        objects: Vec<dos::ObjectId>,
        cursor: Option<String>,
        append: bool,
    },

    /// A write finished; `what` is a human description for the ops log.
    Written {
        what: String,
    },
}

#[derive(Debug)]
pub enum CoreEvent {
    Done {
        req_id: u64,
        result: Result<Payload, ApiError>,
    },
    Tail(TailEvent),
    TailInfo(String),
    TailClosed(String),
}
