//! D1: databases, `/raw` queries (stable column order), schema via
//! `sqlite_master`, Time Travel bookmarks. There is no table-list endpoint.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, Client};

#[derive(Debug, Clone, Deserialize)]
pub struct Database {
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub num_tables: Option<u64>,
    #[serde(default)]
    pub running_in_region: Option<String>,
    #[serde(default)]
    pub read_replication: Option<Value>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
}

pub async fn list(c: &Client, account: &str) -> ApiResult<Vec<Database>> {
    let mut v: Vec<Database> = c
        .get_all_pages(&format!("/accounts/{account}/d1/database"), 1000, &[])
        .await?;
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

pub async fn get(c: &Client, account: &str, id: &str) -> ApiResult<Database> {
    c.get(&format!("/accounts/{account}/d1/database/{id}"), &[])
        .await
}

pub async fn create(
    c: &Client,
    account: &str,
    name: &str,
    location: Option<&str>,
) -> ApiResult<Database> {
    let mut body = json!({ "name": name });
    if let Some(l) = location {
        body["primary_location_hint"] = json!(l);
    }
    c.post(&format!("/accounts/{account}/d1/database"), body)
        .await
}

pub async fn delete(c: &Client, account: &str, id: &str) -> ApiResult<Value> {
    c.delete(&format!("/accounts/{account}/d1/database/{id}"))
        .await
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub changes: Option<u64>,
    #[serde(default)]
    pub last_row_id: Option<i64>,
    #[serde(default)]
    pub rows_read: Option<u64>,
    #[serde(default)]
    pub rows_written: Option<u64>,
    #[serde(default)]
    pub size_after: Option<u64>,
    #[serde(default)]
    pub served_by_region: Option<String>,
    #[serde(default)]
    pub served_by_primary: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RawResults {
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawResult {
    #[serde(default)]
    pub results: RawResults,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub meta: Meta,
}

/// One entry per statement in `sql` (statements split on `;`).
pub async fn raw(
    c: &Client,
    account: &str,
    id: &str,
    sql: &str,
    params: &[Value],
) -> ApiResult<Vec<RawResult>> {
    c.post(
        &format!("/accounts/{account}/d1/database/{id}/raw"),
        json!({ "sql": sql, "params": params }),
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Table {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub sql: Option<String>,
}

pub async fn tables(c: &Client, account: &str, id: &str) -> ApiResult<Vec<Table>> {
    let res = raw(
        c,
        account,
        id,
        "SELECT name, type, sql FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%' ORDER BY type, name;",
        &[],
    )
    .await?;
    let mut out = Vec::new();
    if let Some(first) = res.into_iter().next() {
        for row in first.results.rows {
            let name = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let kind = row
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or("table")
                .to_string();
            let sql = row.get(2).and_then(Value::as_str).map(str::to_string);
            if !name.is_empty() {
                out.push(Table { name, kind, sql });
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub kind: String,
    pub notnull: bool,
    pub pk: bool,
    pub default: Option<String>,
}

pub async fn columns(c: &Client, account: &str, id: &str, table: &str) -> ApiResult<Vec<Column>> {
    let res = raw(
        c,
        account,
        id,
        &format!(
            "SELECT name, type, \"notnull\", pk, dflt_value FROM pragma_table_info({});",
            quote_ident(table)
        ),
        &[],
    )
    .await?;
    let mut out = Vec::new();
    if let Some(first) = res.into_iter().next() {
        for row in first.results.rows {
            out.push(Column {
                name: row
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                kind: row.get(1).and_then(Value::as_str).unwrap_or("").to_string(),
                notnull: row.get(2).and_then(Value::as_i64).unwrap_or(0) != 0,
                pk: row.get(3).and_then(Value::as_i64).unwrap_or(0) != 0,
                default: row.get(4).and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    Ok(out)
}

pub async fn count(c: &Client, account: &str, id: &str, table: &str) -> ApiResult<u64> {
    let res = raw(
        c,
        account,
        id,
        &format!("SELECT COUNT(*) FROM {};", quote_ident(table)),
        &[],
    )
    .await?;
    Ok(res
        .first()
        .and_then(|r| r.results.rows.first())
        .and_then(|row| row.first())
        .and_then(Value::as_u64)
        .unwrap_or(0))
}

#[derive(Debug, Deserialize)]
struct Bookmark {
    bookmark: String,
}

pub async fn bookmark(
    c: &Client,
    account: &str,
    id: &str,
    timestamp: Option<&str>,
) -> ApiResult<String> {
    let q: Vec<(&str, String)> = timestamp
        .map(|t| vec![("timestamp", t.to_string())])
        .unwrap_or_default();
    let b: Bookmark = c
        .get(
            &format!("/accounts/{account}/d1/database/{id}/time_travel/bookmark"),
            &q,
        )
        .await?;
    Ok(b.bookmark)
}

/// Restore to a bookmark or timestamp (query string, empty body).
pub async fn restore(
    c: &Client,
    account: &str,
    id: &str,
    bookmark: Option<&str>,
    timestamp: Option<&str>,
) -> ApiResult<Value> {
    let mut q: Vec<(&str, String)> = Vec::new();
    if let Some(b) = bookmark {
        q.push(("bookmark", b.to_string()));
    }
    if let Some(t) = timestamp {
        q.push(("timestamp", t.to_string()));
    }
    c.request(
        reqwest::Method::POST,
        &format!("/accounts/{account}/d1/database/{id}/time_travel/restore"),
        &q,
        crate::api::Body::None,
    )
    .await
}

/// Double-quote an identifier for SQLite.
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Single-quote a string literal for SQLite.
pub fn quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// SQL literal for a JSON value as D1 returns it.
pub fn literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => {
            if *b {
                "1".into()
            } else {
                "0".into()
            }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote_str(s),
        other => quote_str(&other.to_string()),
    }
}

/// Build an UPDATE for one row keyed by the primary key (or all columns when
/// there is no PK, which is what the dashboard does too).
pub fn update_sql(table: &str, cols: &[Column], old: &[Value], new: &[Value]) -> Option<String> {
    let mut sets = Vec::new();
    for (i, col) in cols.iter().enumerate() {
        if old.get(i) != new.get(i) {
            sets.push(format!(
                "{} = {}",
                quote_ident(&col.name),
                literal(new.get(i).unwrap_or(&Value::Null))
            ));
        }
    }
    if sets.is_empty() {
        return None;
    }
    Some(format!(
        "UPDATE {} SET {} WHERE {};",
        quote_ident(table),
        sets.join(", "),
        where_clause(cols, old)
    ))
}

pub fn delete_sql(table: &str, cols: &[Column], row: &[Value]) -> String {
    format!(
        "DELETE FROM {} WHERE {};",
        quote_ident(table),
        where_clause(cols, row)
    )
}

pub fn insert_sql(table: &str, cols: &[Column], row: &[Value]) -> String {
    let names: Vec<String> = cols.iter().map(|c| quote_ident(&c.name)).collect();
    let vals: Vec<String> = row.iter().map(literal).collect();
    format!(
        "INSERT INTO {} ({}) VALUES ({});",
        quote_ident(table),
        names.join(", "),
        vals.join(", ")
    )
}

fn where_clause(cols: &[Column], row: &[Value]) -> String {
    let pks: Vec<(usize, &Column)> = cols.iter().enumerate().filter(|(_, c)| c.pk).collect();
    let keyed: Vec<(usize, &Column)> = if pks.is_empty() {
        cols.iter().enumerate().collect()
    } else {
        pks
    };
    keyed
        .into_iter()
        .map(|(i, c)| {
            let v = row.get(i).unwrap_or(&Value::Null);
            if v.is_null() {
                format!("{} IS NULL", quote_ident(&c.name))
            } else {
                format!("{} = {}", quote_ident(&c.name), literal(v))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<Column> {
        vec![
            Column {
                name: "id".into(),
                kind: "INTEGER".into(),
                notnull: true,
                pk: true,
                default: None,
            },
            Column {
                name: "name".into(),
                kind: "TEXT".into(),
                notnull: false,
                pk: false,
                default: None,
            },
        ]
    }

    #[test]
    fn update_only_changed() {
        let old = vec![json!(1), json!("a")];
        let new = vec![json!(1), json!("b'c")];
        let sql = update_sql("users", &cols(), &old, &new).unwrap();
        assert_eq!(
            sql,
            "UPDATE \"users\" SET \"name\" = 'b''c' WHERE \"id\" = 1;"
        );
        assert!(update_sql("users", &cols(), &old, &old).is_none());
    }

    #[test]
    fn delete_uses_pk() {
        let row = vec![json!(7), json!(null)];
        assert_eq!(
            delete_sql("t", &cols(), &row),
            "DELETE FROM \"t\" WHERE \"id\" = 7;"
        );
    }

    #[test]
    fn no_pk_uses_all_columns() {
        let c = vec![Column {
            name: "a".into(),
            kind: "".into(),
            notnull: false,
            pk: false,
            default: None,
        }];
        assert_eq!(
            delete_sql("t", &c, &[json!(null)]),
            "DELETE FROM \"t\" WHERE \"a\" IS NULL;"
        );
    }
}
