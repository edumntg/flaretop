# Cloudflare storage products over HTTP — what a TUI can and cannot do

Researched 2026-10-05. Sources: public OpenAPI schema (`cloudflare/api-schemas` `openapi.json`), rendered docs under https://developers.cloudflare.com/api/, product docs, wrangler source on `main` of `cloudflare/workers-sdk`, and read-only probes against a real account with a scoped API token (GETs, D1 `SELECT`s, GraphQL). Where a line says "verified live", the request was actually made and the shape is what came back.

Conventions used below:
- Base: `https://api.cloudflare.com/client/v4`. Auth: `Authorization: Bearer <token>`.
- Envelope: `{ success, errors: [{code, message}], messages: [], result, result_info? }`. A missing permission comes back as **HTTP 403, code 10000 "Authentication error"** with a `documentation_url` (verified live on Vectorize and Hyperdrive, which the probe token could not read).
- Global REST rate limit: 1,200 requests / 5 min per user (response headers `ratelimit: "default";r=1199;t=1`, `ratelimit-policy: "default";q=1200;w=300` — verified live; the R2 objects route reports a separate policy `rtwo_gw_apigw`, same budget). https://developers.cloudflare.com/fundamentals/api/reference/limits/
- GraphQL analytics: `POST https://api.cloudflare.com/client/v4/graphql` (GET not allowed), body `{query, variables}`, 300 queries / 5 min, 31-day retention for the datasets below. https://developers.cloudflare.com/analytics/graphql-api/limits/

Verdict summary (details in each section):

| Product | Verdict | One-line reason |
|---|---|---|
| D1 | **FULL** | Everything the dashboard does is `/query` + `/raw` + control plane + GraphQL; export/import/Time Travel are API-only features |
| Workers KV | **FULL** | Namespaces, keys, values, metadata, bulk all public; only "exact key count" is missing everywhere (dashboard included) |
| R2 | **FULL** (buckets + objects ≤300 MB) / PARTIAL for >300 MB uploads | Object list/get/put/delete now public REST with the account token; multipart and >300 MB need the S3 API and an R2 access key (which can itself be minted via the API) |
| Queues | **PARTIAL** | Everything except the dashboard's non-consuming "peek"; pull/ack needs an `http_pull` consumer that cannot coexist with a Worker consumer |
| Durable Objects | **PARTIAL (read-only inventory)** | List namespaces + object ids only; no storage read, no delete, no alarms |
| Vectorize | **FULL** | v2 index CRUD + vector ops + metadata indexes are all REST; needs Vectorize Read/Write on the token |
| Hyperdrive | **FULL** | Config CRUD is REST; no "test connection" endpoint, but creation itself validates the origin |
| Analytics Engine | **FULL (read)** | SQL API with `SHOW TABLES`; no REST dataset CRUD exists for anyone |

---

## 1. D1

Docs index: https://developers.cloudflare.com/api/resources/d1/ → https://developers.cloudflare.com/api/resources/d1/subresources/database/ (methods `list, create, get, delete, update, edit, query, raw, export, import`; Time Travel under `.../subresources/time_travel/methods/get_bookmark/` and `.../restore/`). Token groups: `D1 Read` for list/get/query/raw; `D1 Write` for create/delete/update/import/restore.

The OpenAPI schema contains exactly these paths under `/d1/`:

```
GET    /accounts/{account_id}/d1/database
POST   /accounts/{account_id}/d1/database
GET    /accounts/{account_id}/d1/database/{database_id}
PUT    /accounts/{account_id}/d1/database/{database_id}
PATCH  /accounts/{account_id}/d1/database/{database_id}
DELETE /accounts/{account_id}/d1/database/{database_id}
POST   /accounts/{account_id}/d1/database/{database_id}/query
POST   /accounts/{account_id}/d1/database/{database_id}/raw
POST   /accounts/{account_id}/d1/database/{database_id}/export
POST   /accounts/{account_id}/d1/database/{database_id}/import
GET    /accounts/{account_id}/d1/database/{database_id}/time_travel/bookmark
POST   /accounts/{account_id}/d1/database/{database_id}/time_travel/restore
```

### 1.1 List — `GET /accounts/{account_id}/d1/database`
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/list/
- Query: `name` (search; verified live it is a **substring** match — `?name=curve` returned `holdcurve`), `page` (default 1), `per_page` (default 1000, min 10, max 10000).
- `result[]`: `{ uuid, name, created_at, version ("alpha"|"beta"|"production"), jurisdiction (null|"eu"|"fedramp"|"us") }`. Verified live the list item also carries `num_tables` and `file_size` (undocumented in the list schema; the first item came back as `{"uuid":..., "name":"holdcurve", "created_at":"2026-10-01T22:21:49.477Z", "version":"production", "num_tables":0, "file_size":221184, "jurisdiction":null}`) — but `num_tables` was **0 in the list and 18 in the GET** for the same database, so do not trust the list-item value.
- `result_info`: `{ count, page, per_page, total_count }` (verified live, `total_count: 13`).
- Wrangler `list.ts` pages with `per_page=10&page=N` until a short page.

### 1.2 Create — `POST /accounts/{account_id}/d1/database`
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/create/
- Body: `{ name (^[a-zA-Z0-9][a-zA-Z0-9_-]*$), primary_location_hint? ("wnam"|"enam"|"weur"|"eeur"|"apac"|"oc"), jurisdiction? ("eu"|"fedramp"|"us" — if present the location hint is ignored), read_replication?: { mode: "auto"|"disabled" } }`.
- Returns the details object (1.3). Wrangler also reads an undocumented `created_in_region`. Error codes: **7502** name already exists, **7406** database quota reached (wrangler `create.ts`).

### 1.3 Get — `GET /accounts/{account_id}/d1/database/{database_id}`
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/get/
- `database_id` accepts a UUID **or the database name** (schema `oneOf`); wrangler resolves names with `GET .../d1/database/{encodeURIComponent(name)}?fields=uuid,name`.
- Query `fields`: comma list of `uuid, name, created_at, version, jurisdiction, num_tables, file_size, running_in_region, read_replication`.
- Verified live result: `{ uuid, name, created_at, version, num_tables: 18, file_size: 221184, running_in_region: "WEUR", read_replication: { mode: "disabled" }, jurisdiction: null }`.
- The schema marks `num_tables` **deprecated: "This count is no longer accurate and should not be relied upon"** — compute it from `sqlite_master`.
- **There are no `read_queries_24h` / `rows_read_24h` etc. in REST.** `wrangler d1 info` gets them from GraphQL and merges (1.10).

### 1.4 Delete — `DELETE .../d1/database/{database_id}` → `result: null`. https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/delete/

### 1.5 Read replication — `PUT` (full) / `PATCH` (partial) `.../d1/database/{database_id}`
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/update/ and `.../methods/edit/`; product doc https://developers.cloudflare.com/d1/best-practices/read-replication/
- Body: `{ "read_replication": { "mode": "auto" | "disabled" } }` — the only updatable property. Returns the details object. Disabling can take up to 24 h for replicas to be removed.

### 1.6 `/query` vs `/raw`
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/query/ and `.../methods/raw/`

Request body is identical for both:
```json
{ "sql": "SELECT * FROM t WHERE a = ?;", "params": ["x"] }
```
or `{ "batch": [ { "sql", "params" }, ... ] }` (in the schema; whether `batch` is transactional like the binding's `batch()` is unanswered — https://github.com/cloudflare/workers-sdk/issues/15904). `params` is typed as array of strings. The `sql` string "supports multiple statements, joined by semicolons, which will be executed as a batch" — the server splits on `;` and returns one result per statement (verified live: two statements → two entries in `result`).

`/query` response — `result` is an **array** with one entry per statement, rows as objects (verified live):
```json
{ "result": [
    { "results": [ {"name":"_cf_KV","type":"table"}, ... ],
      "success": true,
      "meta": { "served_by":"v3-prod", "served_by_region":"WEUR", "served_by_colo":"AMS",
                "served_by_primary":true, "timings":{"sql_duration_ms":0.59}, "duration":0.59,
                "changes":0, "last_row_id":0, "changed_db":false, "size_after":221184,
                "rows_read":4, "rows_written":0, "total_attempts":1 } },
    { "results": [ {"x":1} ], "success": true, "meta": { ... } } ],
  "errors": [], "messages": [], "success": true }
```
`/raw` response — same, but `results` is `{ "columns": [...], "rows": [[...], ...] }` (verified live: `{"columns":["name","type"],"rows":[["_cf_KV","table"],["d1_migrations","table"]]}`). The docs call `/raw` "a performance-optimized version of the /query endpoint". For a table browser use `/raw`: stable column order and columns present even with zero rows.

Error shape (verified live, HTTP 400): `{"messages":[],"result":[],"success":false,"errors":[{"code":7500,"message":"no such table: no_such_table: SQLITE_ERROR"}]}`.

`meta` fields: `changed_db, changes, duration (ms in DB), last_row_id, rows_read (incl. index reads), rows_written, served_by_colo, served_by_primary, served_by_region (WNAM|ENAM|WEUR|EEUR|APAC|OC), size_after (bytes), timings.sql_duration_ms, total_attempts`.

**Limits** (https://developers.cloudflare.com/d1/platform/limits/, updated 2026-04-21): max SQL statement length **100,000 bytes**; max bound parameters **100** per statement; max string/BLOB/row **2,000,000 bytes**; max 100 columns/table; **30 s** wall clock "applies to the entire batch call" (API requests must resolve in 30 s); 10 GB/db Paid (500 MB Free); 50,000 dbs/account Paid (10 Free). "Limits for individual queries apply to each individual statement contained within a batch statement." There is **no documented statement-count cap, response-size cap or 100 MB body cap** for `/query` — the "50 statements"/"100 statements" figures circulating are not in Cloudflare docs; the real ceiling is the 30 s timeout. Historical note: wrangler ≤3.x split files into 10,000-statement chunks (`DEFAULT_BATCH_SIZE`); wrangler 4 removed that and routes files through `/import`.

### 1.7 Export — `POST .../export` (async polling)
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/export/
- Body: `{ "output_format": "polling", "dump_options"?: { "no_schema"?, "no_data"?, "tables"?: [..] }, "current_bookmark"?: string }`.
- HTTP **202** while running: `result: { success, type: "export", status: "active", at_bookmark, messages: [string] }`.
- HTTP **200** when finished: `result: { status: "complete", at_bookmark, messages, result: { filename, signed_url } }` (signed_url valid one hour) or `status: "error"` with `error`.
- Flow used by wrangler `export.ts` (`pollExport`): POST once with no bookmark → if `status == "active"` POST again **immediately** with `current_bookmark = at_bookmark` (same `dump_options`), no sleep → on `complete` `GET signed_url` and save; on `error` throw. The schema warns the DB is unavailable during export and "an in-progress export must be continually polled or will automatically cancel". `messages` like `"Uploaded part N"` arrive out of order. FTS5/virtual tables are not exported (https://developers.cloudflare.com/d1/best-practices/import-export-data/).

### 1.8 Import — `POST .../import` (three actions)
https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/import/
1. `{ "action": "init", "etag": "<md5 hex of the .sql file>" }` → `result: { filename, upload_url }` (R2 presigned PUT). If a file with that etag already exists the import starts immediately and you get a polling response instead (wrangler checks `"upload_url" in result`).
2. `PUT <upload_url>` with the raw file and `Content-Length`; expect 200 and response header `etag` == your md5 (strip quotes).
3. `{ "action": "ingest", "etag", "filename" }` → polling response.
4. `{ "action": "poll", "current_bookmark" }` repeated while `status == "active"`.
- Polling response: 202 `{ status:"active", at_bookmark, messages }`; 200 `{ status:"complete"|"error", at_bookmark, messages, errors?: [string], result?: { final_bookmark, num_queries, meta: {...} } }`.
- Imports block the database. Max file 5 GB. Strip `BEGIN TRANSACTION`/`COMMIT` and the `_cf_KV` table from `.dump` output.

### 1.9 Time Travel
https://developers.cloudflare.com/d1/reference/time-travel/ ; API: `.../subresources/time_travel/methods/get_bookmark/` and `.../restore/`
- `GET .../time_travel/bookmark[?timestamp=<ISO 8601>]` → `result: { bookmark }` (verified live: `"00000009-00000000-000050fb-6111…"`; with `?timestamp=2026-10-04T00:00:00Z` a different, earlier bookmark). No timestamp = current bookmark; with timestamp = nearest bookmark at or before. Bookmarks sort lexicographically.
- `POST .../time_travel/restore?bookmark=…` **or** `?timestamp=…` — **query string, empty body** (OpenAPI; wrangler `restore.ts` sends no body) → `result: { bookmark, previous_bookmark, message }`. `previous_bookmark` lets you undo.
- Limits: 30 days Paid / 7 days Free; **10 restores per 10 min per database**; wrangler refuses `version: "alpha"` databases and timestamps in the future or >30 days old.

### 1.10 Metrics (GraphQL) — what `wrangler d1 info` and the dashboard actually use
Datasets (names and fields verified live, https://developers.cloudflare.com/d1/observability/metrics-analytics/):
- `d1AnalyticsAdaptiveGroups` — `sum { readQueries writeQueries rowsRead rowsWritten queryBatchResponseBytes }`, `avg { queryBatchTimeMs queryBatchResponseBytes }`, `quantiles { queryBatchTimeMsP50 … P999 }`, `count`, `dimensions { databaseId databaseRole servedByRegion servedByInstance date datetime datetimeMinute datetimeFiveMinutes datetimeFifteenMinutes datetimeHour datetimeSixHours }`. Verified live: `filter:{date_geq:"2026-09-28", date_leq:"2026-10-05"}` grouped by `databaseId` returns `sum { readQueries rowsRead rowsWritten writeQueries }`.
- `d1QueriesAdaptiveGroups` — per-query: `dimensions { query error databaseId … }`, `sum/avg { queryDurationMs rowsRead rowsWritten rowsReturned }`, `count`.
- `d1StorageAdaptiveGroups` — `max { databaseSizeBytes }`, `dimensions { databaseId date datetime… }` (verified live).

Wrangler `info.ts` query (verbatim):
```graphql
query getD1MetricsOverviewQuery($accountTag: string, $filter: ZoneWorkersRequestsFilter_InputObject) {
  viewer { accounts(filter: {accountTag: $accountTag}) {
    d1AnalyticsAdaptiveGroups(limit: 10000, filter: $filter) {
      sum { readQueries writeQueries rowsRead rowsWritten }
      dimensions { datetimeHour } } } } }
```
variables `{ accountTag, filter: { AND: [{ datetimeHour_geq: <now-24h>, datetimeHour_leq: <now>, databaseId }] } }`; wrangler sums the rows into `read_queries_24h` etc., renames `file_size` → `database_size`. `wrangler d1 insights` uses `d1QueriesAdaptiveGroups(limit, filter, orderBy:[count_DESC | sum_queryDurationMs_DESC | …])` with `sum/avg { queryDurationMs rowsRead rowsWritten rowsReturned } count dimensions { query }`.

### 1.11 Wrangler → endpoint map (packages/wrangler/src/d1/)
| Command | Call |
|---|---|
| `d1 list` | `GET .../d1/database?per_page=10&page=N` |
| `d1 create` | `POST .../d1/database` |
| `d1 delete` | name→uuid via `GET .../d1/database/{name}?fields=uuid,name`, then `DELETE` |
| `d1 info` | `GET .../d1/database/{uuid}` + GraphQL above |
| `d1 execute --remote --command` | **`POST .../query`** with `{sql}` only — no params, no client-side chunking; `/raw` is never used by wrangler |
| `d1 execute --remote --file` | `/import` init → PUT upload → ingest → poll |
| `d1 export --remote` | `/export` polling loop |
| `d1 time-travel info/restore` | `GET .../time_travel/bookmark`, `POST .../time_travel/restore?bookmark=` |
| `d1 migrations *` | plain SQL via `/query` on table `d1_migrations` |
| `d1 backups` | **removed** — alpha snapshot backups API shut down 2025-07-01 (https://developers.cloudflare.com/d1/reference/backups/) |
Non-public: wrangler adds `x-d1-internal-env` header when `database_internal_env` is configured (Cloudflare-internal only).

### 1.12 List tables — confirmed: no endpoint
Nothing in the schema enumerates tables/columns/indexes. Use `/raw`:
```sql
SELECT name, type, sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%' ORDER BY name;
```
(verified live; results include `_cf_KV`, `d1_migrations`, `sqlite_sequence` which you should hide; `_cf_KV` is reserved and cannot be queried). Columns: `SELECT * FROM pragma_table_info('t')`. The dashboard's own Tables tab runs `SELECT tbl_name as name, (SELECT ncol FROM pragma_table_list(tbl_name)) as num_columns FROM sqlite_master WHERE TYPE = "table" AND tbl_name NOT LIKE "sqlite_%" AND tbl_name NOT LIKE "d1_%" AND tbl_name NOT LIKE "_cf_%" ORDER BY tbl_name ASC;` (visible in `d1 insights` output per the docs).

### 1.13 Dashboard vs API
Dashboard tabs: Console (= `/query`), Tables (= `sqlite_master` queries + `/query` edits), Metrics (= GraphQL), Settings (= read replication PUT). Nothing is dashboard-only. The API additionally offers export/import/Time Travel. **Verdict: FULL.**

---

## 2. Workers KV

Docs: https://developers.cloudflare.com/api/resources/kv/ ; limits https://developers.cloudflare.com/kv/platform/limits/ ; wrangler source https://raw.githubusercontent.com/cloudflare/workers-sdk/main/packages/wrangler/src/kv/helpers.ts . Permissions: `Workers KV Storage Read` for GET/list/bulk-get, `Workers KV Storage Write` for the rest.

### 2.1 Namespaces — `/accounts/{account_id}/storage/kv/namespaces`
| Op | Call | Notes |
|---|---|---|
| List | `GET …/namespaces?page=&per_page=&order=id\|title&direction=asc\|desc` | `per_page` min 1, max 1000. `result_info: { count, page, per_page, total_count, total_pages }` (verified live). Wrangler uses `per_page=100&order=title&direction=asc`. |
| Create | `POST …/namespaces` body `{ title (≤512), jurisdiction? ("eu"\|"us"\|"fedramp"), mode?: "instant" }` | jurisdiction only at creation |
| Get | `GET …/namespaces/{id}` | verified: `{ id, title, supports_url_encoding: true }` |
| Rename | `PUT …/namespaces/{id}` body `{ title }` | |
| Delete | `DELETE …/namespaces/{id}` | `result: {}` |
https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/methods/list/ (…/create/, …/get/, …/update/, …/delete/)

### 2.2 Keys — `GET …/namespaces/{id}/keys?limit=&prefix=&cursor=`
https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/subresources/keys/methods/list/
- `limit`: **min 10, max 1000** (verified live: `limit=5` → code 10028 "limit argument must be at least 10"; `limit=5000` → "must be no greater than 1000"); default 1000.
- `result[]`: `{ name, expiration?: unix secs, metadata?: any }`; `result_info: { count, cursor }`. End of list = cursor absent/null/**empty string** (verified live: last page has `"cursor": ""`). Wrangler's `hasCursor()` checks all three.
- No total count anywhere in REST.

### 2.3 Values and metadata
- `GET …/values/{key}` → raw bytes, `content-type: application/octet-stream`, `accept-ranges: bytes`, `cache-control: no-store` (verified live). Metadata is **not** returned here (no header); the only extra header is `expiration` (unix seconds) when the key has one. Missing key → HTTP 404 with a **non-standard body** `{"code": 1000, "error": "not_found"}` (verified live) — handle both envelope shapes. Percent-encode the key.
- `GET …/metadata/{key}` → `result` is the metadata JSON itself, `null` when none (verified live). https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/subresources/metadata/methods/get/
- `PUT …/values/{key}?expiration=<unix>&expiration_ttl=<secs ≥ 60>` — `multipart/form-data` with fields `value` (≤ 25 MiB) and `metadata` (JSON string ≤ 1024 B). `expiration_ttl` wins over `expiration`. `result: {}`. Wrangler sends the raw value as the body when there is no metadata (undocumented shortcut that works). https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/subresources/values/methods/update/
- `DELETE …/values/{key}` → `result: {}`.

### 2.4 Bulk
- Write: `PUT …/namespaces/{id}/bulk` — body is a bare JSON **array** of `{ key, value (string), base64?: bool, expiration?, expiration_ttl? (≥60), metadata? }`, max **10,000** pairs / **100 MB** body → `result: { successful_key_count, unsuccessful_keys: [] }`. https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/methods/bulk_update/
- Delete: documented form is **`POST …/bulk/delete`** with a bare array of key names (max 10,000); wrangler still uses legacy `DELETE …/bulk` with the same array — both live. https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/methods/bulk_delete/
- Get: `POST …/bulk/get` body `{ keys: [..] (max **100**, verified live: 101 → code 10029 "You can request a maximum of 100 keys"), type?: "text"|"json", withMetadata?: bool }`. Verified live response: without metadata `result: { values: { "k": "<string>", "missing": null } }`; with `withMetadata: true` `result: { values: { "k": { value, metadata: null|{...}, expiration? }, "missing": null } }`. https://developers.cloudflare.com/api/resources/kv/subresources/namespaces/methods/bulk_get/
- Wrangler constants: `API_MAX = 10000`, `BATCH_KEY_MAX = 1000` (not 5000).

### 2.5 Limits
Key 512 B; value 25 MiB; metadata 1024 B; 1 write/s per key; 1,000 namespaces/account; min TTL 60 s; Free 100k reads + 1k writes/day, 1 GB; eventually consistent (~60 s). https://developers.cloudflare.com/kv/platform/limits/

### 2.6 Metrics (GraphQL, verified live)
- `kvStorageAdaptiveGroups` — `max { keyCount byteCount }`, `dimensions { namespaceId date … }` (daily snapshot; note `namespaceId` comes back **hyphenated UUID**, e.g. `4656816e-31f1-…`, while REST ids are 32 hex chars — normalise before matching).
- `kvOperationsAdaptiveGroups` — `sum { requests objectBytes }`, `quantiles { latencyMsP50 … }`, `dimensions { namespaceId actionType responseStatusCode result … }`.
https://developers.cloudflare.com/kv/observability/metrics-analytics/

### 2.7 Dashboard vs API
The dashboard key browser is the same `/keys` + `/values` calls. Not possible anywhere (dashboard included): exact live key count, search by value, server-side sort. **Verdict: FULL.**

---

## 3. R2

Docs root https://developers.cloudflare.com/api/resources/r2/ ; limits https://developers.cloudflare.com/r2/platform/limits/ (REST: **1,200 req / 5 min account-wide across all R2 REST ops**; wrangler's bulk uploader budgets 1,100). Wrangler helpers: `packages/wrangler/src/r2/helpers/{bucket,object,domain,notification,catalog}.ts`.

Common headers: `cf-r2-jurisdiction: default|eu|us|fedramp|fedramp-high` (jurisdictional buckets are invisible without it — verified live: `GET /r2/buckets` with `cf-r2-jurisdiction: eu` returned `{"buckets":[]}` while the default call listed 3+), `cf-r2-storage-class: Standard|InfrequentAccess`, `cf-r2-data-catalog-check: true` (mutations fail 409 / code **10081** if Data Catalog is enabled; wrangler sends it unless `--force`).

### 3.1 Buckets — `/accounts/{account_id}/r2/buckets`
- List `GET …/r2/buckets?name_contains=&start_after=&per_page=(default 20, max 1000)&order=name&direction=asc|desc&cursor=` → verified live `result: { buckets: [{ name, creation_date }] }`, `result_info: { cursor, per_page }` (list items carry only name+date; `location`/`storage_class`/`jurisdiction` need the GET). https://developers.cloudflare.com/api/resources/r2/subresources/buckets/methods/list/
- Create `POST …/r2/buckets` body `{ name, locationHint? (apac|eeur|enam|weur|wnam|oc), storageClass? }`; jurisdiction via header; wrangler refuses location + jurisdiction together. Also `PUT …/r2/buckets/{name}` (no body, no location hint).
- Get `GET …/r2/buckets/{name}` → verified live `{ name, creation_date, location: "EEUR", storage_class: "Standard", jurisdiction: "default" }`.
- Storage class `PATCH …/r2/buckets/{name}` with header `cf-r2-storage-class` (existing objects keep theirs).
- Delete `DELETE …/r2/buckets/{name}` — must be empty, no in-progress multipart, no event-notification rules.

### 3.2 Bucket sub-resources (all `…/r2/buckets/{name}/…`)
| Resource | Methods | Shape (verified live where noted) |
|---|---|---|
| `cors` | GET/PUT/DELETE | `{ rules: [{ id?, allowed: { origins[], methods[] (GET\|PUT\|POST\|DELETE\|HEAD), headers?[] }, exposeHeaders?[], maxAgeSeconds? }] }`. GET with none configured → **HTTP 404 code 10059** "The CORS configuration does not exist." (live) |
| `lifecycle` | GET/PUT | `{ rules: [{ id, enabled, conditions: { prefix }, deleteObjectsTransition?: { condition: {type:"Age", maxAge: secs} \| {type:"Date", date} }, storageClassTransitions?: [{ condition, storageClass:"InfrequentAccess" }], abortMultipartUploadsTransition?: { condition } }] }`. Live default: `{"id":"Default Multipart Abort Rule","enabled":true,"conditions":{},"abortMultipartUploadsTransition":{"condition":{"type":"Age","maxAge":604800}}}`. Ages are **seconds**; max 1000 rules. https://developers.cloudflare.com/r2/buckets/object-lifecycles/ |
| `domains/custom` | GET list; POST `{ domain, zoneId, enabled, minTLS?, ciphers? }`; GET/PUT/DELETE `…/custom/{domain}` | list → `{ domains: [{ domain, enabled, status: { ownership, ssl }, minTLS, ciphers, zoneId, zoneName }] }` (live: `{"domains":[]}`); zone must be in the same account; ≤100/bucket |
| `domains/managed` | GET; PUT `{ enabled }` | live: `{ enabled: false, bucketId, domain: "pub-<bucketId>.r2.dev" }`. **This is the dashboard's "public access" toggle.** |
| event notifications | **not under `/r2/buckets`** — `GET /accounts/{id}/event_notifications/r2/{bucket}/configuration`; `GET/PUT/DELETE …/configuration/queues/{queue_id}` | PUT `{ rules: [{ actions: (PutObject\|CopyObject\|DeleteObject\|CompleteMultipartUpload\|LifecycleDeletion)[], prefix?, suffix?, description? }] }` (adds; overlapping rules → 409); DELETE body optional `{ ruleIds }`. Live with none: HTTP 404 code 11015 "No event notification config found"; `GET …/r2/buckets/{b}/event_notifications` → 404 code 10015 "No route". ≤100 rules. https://developers.cloudflare.com/r2/buckets/event-notifications/ |
| `sippy` | GET/PUT/DELETE | PUT `{ source: {provider:"aws", region, bucket, accessKeyId, secretAccessKey} \| {provider:"gcs", bucket, clientEmail, privateKey} \| {provider:"s3", bucketUrl, …} \| {provider:"azure", …}, destination: { provider:"r2", accessKeyId, secretAccessKey } }` — destination creds are an **R2 access key pair**, there is no `cloudApiToken` field. Live GET: `{ enabled: false }`. https://developers.cloudflare.com/r2/data-migration/sippy/ |
| `lock` | GET/PUT/DELETE | `{ rules: [{ id, enabled, prefix?, condition: {type:"Age", maxAgeSeconds} \| {type:"Date", date} \| {type:"Indefinite"} }] }` (live: `{"rules":[]}`); ≤1000 rules. https://developers.cloudflare.com/r2/buckets/bucket-locks/ |
| `jobs` | POST `{ jobType:"prefixDelete", prefix }` or `{ jobType:"storageClassMigration", sourceStorageClass?, destinationStorageClass? }`; `GET …/jobs?jobType=&status=&maxKeys=&continuationToken=`; `GET …/jobs/{id}` | background jobs: prefix delete (prefix must end in `/`, `""` empties the bucket), storage-class migration; status `ENQUEUED\|RUNNING\|COMPLETED\|FAILED\|CANCELLED` |
| `local-uploads` | GET; PUT `{ enabled }` | default jurisdiction only |

### 3.3 Metrics
- `GET /accounts/{account_id}/r2/metrics` — **account-wide only**. Verified live: `{ standard: { uploaded: { objects, payloadSize, metadataSize }, published: { objects: 601, payloadSize: 108381883, metadataSize: 34872 } }, infrequentAccess: { uploaded: {...}, published: {...} } }`.
- **Per-bucket size/count is GraphQL only.** Verified live: `r2StorageAdaptiveGroups(limit, filter:{datetime_geq, datetime_leq[, bucketName]}) { dimensions { bucketName } max { objectCount payloadSize metadataSize uploadCount } }`. Wrangler `r2 bucket info` runs exactly this with `limit: 1, orderBy: [datetime_DESC]` over the last 24 h; for jurisdictional buckets `bucketName = "<jurisdiction>_<name>"`. Also `r2OperationsAdaptiveGroups { dimensions { actionType actionStatus bucketName objectName responseStatusCode } sum { requests } }` and `r2BandwidthUsageAdaptiveGroups { sum { bytesUpload bytesDownload } }`. https://developers.cloudflare.com/r2/platform/metrics-analytics/

### 3.4 Objects over the Cloudflare REST API — public now, works with the account token
The `/objects` routes are **in the public OpenAPI schema** (operationIds `r2-list-objects`, `r2-get-object`, `r2-put-object`, `r2-delete-object`, `r2-delete-objects`) with docs pages, e.g. https://developers.cloudflare.com/api/resources/r2/subresources/buckets/subresources/objects/methods/list/ (fetched 2026-10-05). Every page says "For most workloads, we recommend using R2's S3-compatible API or a Worker with an R2 binding instead." Permission: account-scoped **Workers R2 Storage Read/Write** (bucket-scoped "Object Read & Write" tokens work only with S3, per https://developers.cloudflare.com/r2/api/tokens/).

- **List** `GET …/r2/buckets/{bucket}/objects?per_page=(default 20, max 1000)&prefix=&delimiter=&cursor=&start_after=` + `cf-r2-jurisdiction`. Verified live:
  ```json
  { "result": [ { "key": "u/…/d22c…", "etag": "385816e3…", "last_modified": "2026-09-02T03:41:04.839Z",
                  "size": 7369, "http_metadata": { "contentType": "application/octet-stream" },
                  "custom_metadata": {}, "storage_class": "Standard" } ],
    "result_info": { "cursor": "1-JTdC…", "is_truncated": true, "per_page": 2 } }
  ```
  With `delimiter=/` the "folders" come back in `result_info.delimited: ["u/"]` (verified) and `result` holds only the direct children. `per_page=2000` → code 10029 "per_page must be between 0 and 1000" (verified). `etag` is unquoted hex. Parse `result` defensively (a community report saw it as an array, `{objects:[…]}` or a map).
- **Get** `GET …/objects/{key}` → 200 `application/octet-stream` with headers `content-length`, `content-type`, `etag` (quoted), `last-modified`, `cf-r2-storage-class`, `content-disposition: attachment; filename="…"` (verified live, 7369 bytes). Conditional headers `If-None-Match`/`If-Modified-Since` → 304. **HEAD → 405** and **`Range` is ignored** (full 200, verified live) — for metadata-only use the list route with `prefix=<key>`. Missing key → HTTP 404 code 10007 "The specified key does not exist." (verified).
- **Put** `PUT …/objects/{key}` with body = bytes; headers `Content-Type`, `Content-Length`, `cf-r2-jurisdiction`, `cf-r2-storage-class`; **max 300 MB** (schema; wrangler `MAX_UPLOAD_SIZE_BYTES = 300 MiB`) → `result: { key, etag, size, storage_class, uploaded, version }`. No multipart on this route.
- **Delete** `DELETE …/objects/{key}` → `{ key }`. **Bulk delete** `DELETE …/objects` with a JSON array of 1–1000 keys → `result: [{ key }]`.
- Wrangler `helpers/object.ts` (verbatim paths): `getR2Object` → `fetchR2Objects("/accounts/${accountId}/r2/buckets/${bucketName}/objects/${objectName}", {method:"GET"})`; `putRemoteObject` → same path, `{method:"PUT", body, duplex:"half"}`, forwards headers `cache-control, content-disposition, content-encoding, content-language, content-length, content-type, expires` plus `cf-r2-jurisdiction`/`cf-r2-storage-class`/`cf-r2-data-catalog-check`; `deleteR2Object` → same path `DELETE`. Wrangler interpolates the key raw (no `encodeURIComponent`); it has **no `r2 object list`** command (workers-sdk#13008).

### 3.5 S3-compatible API and access keys
- Endpoint `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` (jurisdictions: `<ACCOUNT_ID>.eu.` / `.us.` / `.fedramp.`), region `auto`. Full op list (ListObjectsV2, Head/Get/Put/Copy/DeleteObject(s), multipart, bucket CORS/lifecycle/location) at https://developers.cloudflare.com/r2/api/s3/api/ . Limits: object 5 TiB, single PUT/part 5 GiB, 10,000 parts, key 1,024 B, metadata 8,192 B, 1 write/s per key.
- Presigned URLs: SigV4 query-string, 1 s–**7 days**, GET/HEAD/PUT/DELETE, S3 hostname only. https://developers.cloudflare.com/r2/api/s3/presigned-urls/
- **Documented trick (confirmed)**: "Access Key ID: The `id` of the API token. Secret Access Key: The SHA-256 hash of the API token `value`." https://developers.cloudflare.com/r2/api/tokens/#get-s3-api-credentials-from-an-api-token — applies to tokens created via `POST /user/tokens` (and after `PUT /user/tokens/{id}/value` rolls); using the raw value gives `SignatureDoesNotMatch`.
- Creating the R2 token programmatically: `POST /user/tokens` (https://developers.cloudflare.com/api/resources/user/subresources/tokens/) or account-owned `POST /accounts/{account_id}/tokens` (https://developers.cloudflare.com/api/resources/accounts/subresources/tokens/) with body `{ name, policies: [{ effect:"allow", resources: {...}, permission_groups: [{ id }] }], expires_on?, condition? }` → `{ id, value }` (value shown once). Caller needs **API Tokens Write**. Resources: one bucket `"com.cloudflare.edge.r2.bucket.<ACCOUNT_ID>_<default|eu|…>_<BUCKET>": "*"`; all buckets `"com.cloudflare.api.account.<ACCOUNT_ID>": { "com.cloudflare.edge.r2.bucket.*": "*" }`. Resolve group ids at runtime via `GET /accounts/{account_id}/tokens/permission_groups?name=` (names: `Workers R2 Storage Write/Read`, `Workers R2 Storage Bucket Item Write/Read`, `Workers R2 Data Catalog Write/Read`; the probe token got **403 code 9109** on this endpoint — it needs token-management permission).
- Temporary creds: `POST /accounts/{account_id}/r2/temp-access-credentials` `{ bucket, parentAccessKeyId, permission: admin-read-write|admin-read-only|object-read-write|object-read-only, ttlSeconds (default 900, **max 604800**), objects?[], prefixes?[] }` → `{ accessKeyId, secretAccessKey, sessionToken }`. https://developers.cloudflare.com/r2/api/s3/temporary-credentials/

### 3.6 Other R2 surfaces
- Super Slurper (public, lives under `/slurper`, verified live `GET /accounts/{id}/slurper/jobs` → bare `[]`): `GET/POST …/slurper/jobs`, `GET/DELETE …/jobs/{id}`, `PUT …/jobs/{id}/abort|pause|resume`, `PUT …/jobs/abortAll`, `GET …/jobs/{id}/progress`, `GET …/jobs/{id}/logs`, `PUT …/slurper/source|target/connectivity-precheck`. https://developers.cloudflare.com/api/resources/r2/subresources/super_slurper/
- R2 Data Catalog: `GET /accounts/{id}/r2-catalog`, `GET /r2-catalog/{bucket}`, `POST …/enable|disable|delete`, `…/credential`, `…/maintenance-configs` — needs **Workers R2 Data Catalog Read/Write** (probe token: 403 code 10000). https://developers.cloudflare.com/r2/data-catalog/manage-catalogs/
- Billing: not under R2 permissions; `GET /accounts/{id}/billable-usage` needs Billing Read.

### 3.7 Dashboard vs API, and the verdict
Object browser = public list route; upload = PUT ≤300 MB (the dashboard has the same 300 MB ceiling); public access = managed domain; everything else above. Gaps with only an account token: **uploads > 300 MB / multipart**, **HEAD/Range**, and **presigned URLs** — all need the S3 API with an R2 access key, which the TUI can mint itself if the token has API Tokens Write. **Verdict: FULL for browsing and managing buckets/objects with just an account API token; PARTIAL only for large uploads, which you solve by creating an R2 token and switching to S3.**

---

## 4. Queues

Docs: https://developers.cloudflare.com/api/resources/queues/ ; pull consumers https://developers.cloudflare.com/queues/configuration/pull-consumers/ ; limits https://developers.cloudflare.com/queues/platform/limits/ . Permissions: `Queues Read` for GETs; `Queues Write` for everything else including pull/ack (docs: pull tokens need Queues Edit). Wrangler paths now live in `packages/deploy-helpers/src/triggers/queue-consumers.ts` (`src/queues/client.ts` is a thin wrapper).

### 4.1 Queues — `/accounts/{account_id}/queues`
| Op | Call |
|---|---|
| List | `GET …/queues?page=&per_page=` (+ undocumented `name=<queue_name>` filter, verified live, used by `wrangler queues info`) → `result_info: { count, page, per_page, total_count, total_pages }` |
| Create | `POST …/queues` `{ queue_name, jurisdiction?, settings?: { delivery_delay, message_retention_period } }` |
| Get | `GET …/queues/{queue_id}` |
| Update | `PUT …/queues/{queue_id}` (full) / **`PATCH`** (partial — wrangler uses PATCH for update, pause-delivery, resume-delivery) |
| Delete | `DELETE …/queues/{queue_id}` |
Verified live queue object: `{ queue_id, queue_name, settings: { delivery_delay: 0, message_retention_period: 345600 }, created_on, modified_on, producers_total_count, producers: [{ script, type: "worker" }], consumers_total_count, consumers: [{ script, dead_letter_queue, settings: { batch_size, max_retries, max_wait_time_ms, retry_delay }, consumer_id, type: "worker" }] }`. Pause = `settings.delivery_paused: true` (pulls then return 409 `queue_delivery_paused`). Ranges: `delivery_delay` 0–86400 s; `message_retention_period` 60–1,209,600 s (≤86,400 Free). https://developers.cloudflare.com/queues/configuration/pause-purge/

### 4.2 Consumers — `…/queues/{queue_id}/consumers`
`POST`, `GET` (list; verified live `result[]` with `result_info`), `GET|PUT|DELETE …/consumers/{consumer_id}` (verified live GET by id).
- worker: `{ type:"worker", script_name, environment_name?, dead_letter_queue?, settings: { batch_size, max_retries, max_wait_time_ms, max_concurrency, retry_delay } }`
- http_pull: `{ type:"http_pull", dead_letter_queue?, settings: { batch_size, max_retries, retry_delay, visibility_timeout_ms } }`
Ranges: `batch_size` ≤100 (default 10 worker / 5 pull), `max_wait_time_ms` ≤60,000, `max_retries` ≤100 (default 3), `retry_delay` 0–86,400, `max_concurrency` ≤250, `visibility_timeout_ms` default 30,000 max 43,200,000 (12 h). **An `http_pull` consumer cannot coexist with a worker consumer.** https://developers.cloudflare.com/api/resources/queues/subresources/consumers/methods/create/

### 4.3 Pull API
- `POST …/queues/{queue_id}/messages/pull` `{ visibility_timeout_ms?: (default 30000, max 43200000), batch_size?: (default 5, max 100) }` → `result: { message_backlog_count, messages: [{ id, body, attempts, lease_id, metadata: { "CF-sourceMessageSource", "CF-Content-Type" }, timestamp_ms }] }`. `body` is plain text for `text`, **base64** for `json`/`bytes`. https://developers.cloudflare.com/api/resources/queues/subresources/messages/methods/pull/
- `POST …/messages/ack` `{ acks: [{ lease_id }], retries: [{ lease_id, delay_seconds? (0–86400) }] }` → `{ ackCount, retryCount, warnings: { <lease_id>: string } }`. https://developers.cloudflare.com/api/resources/queues/subresources/messages/methods/ack/
- **Peek without consuming: not available.** A pull leases the message for `visibility_timeout_ms`; pull-then-retry(delay 0) returns it but **spends an `attempts`** each cycle, so after `max_retries` previews the message is dropped/DLQ'd. A TUI "preview" must warn about this and only works when the sole consumer is `http_pull`.

### 4.4 Push over HTTP (public)
- `POST …/queues/{queue_id}/messages` `{ body, content_type?: "text"|"json", delay_seconds? }`; `POST …/messages/batch` `{ delay_seconds?, messages: [{ body, content_type, delay_seconds? }] }`. Only `text`/`json` over HTTP (`bytes`/`v8` are binding-only). Response includes `result.metadata.metrics: { backlog_bytes, backlog_count, oldest_message_timestamp_ms }` — the only exact backlog in REST besides `message_backlog_count` on pull. Limits 128 KB/message, 100 msgs or 256 KB per batch. https://developers.cloudflare.com/api/resources/queues/subresources/messages/methods/push/ and …/bulk_push/

### 4.5 Purge
`POST …/queues/{queue_id}/purge` `{ delete_messages_permanently: true }` → queue object. Status is **`GET …/queues/{queue_id}/purge`** (not `/purge/preview` — that path returns a bare `404 Not Found`; verified live, the GET returns `result: null` when nothing has been purged) → `{ started_at, completed }`. https://developers.cloudflare.com/api/resources/queues/subresources/purge/methods/start/ and …/status/

### 4.6 Metrics (GraphQL, verified live)
`queueBacklogAdaptiveGroups { dimensions { queueId } avg { messages bytes } }` (the docs prose says `queuesBacklogAdaptiveGroups`; the live schema name is **`queueBacklogAdaptiveGroups`**), `queueDelayedBacklogAdaptiveGroups { avg { messages } }`, `queueConsumerMetricsAdaptiveGroups { avg { concurrency } }`, `queueMessageOperationsAdaptiveGroups { dimensions { actionType consumerType outcome queueId } sum { billableOperations bytes } avg { lagTime retryCount } max { messageSize } }`. Idle queues return no backlog rows. https://developers.cloudflare.com/queues/observability/metrics/

### 4.7 Dashboard vs API
The dashboard Messages tab can (a) **send** bytes/file payloads and (b) **list/peek and ack messages without consuming** them, even with a Worker consumer attached (https://developers.cloudflare.com/queues/examples/list-messages-from-dash/). Neither is in the public API. Everything else (queue/consumer CRUD, pause, purge, push text/json, pull/ack, metrics) is. **Verdict: PARTIAL** — full management, no faithful message browser.

---

## 5. Durable Objects

The public REST surface is exactly two endpoints (the TypeScript SDK's `durableObjects.namespaces` has only `list` and `objects.list`; the API reference index matches). Permission: Workers Scripts Read/Write.

- `GET /accounts/{account_id}/workers/durable_objects/namespaces?page=&per_page=(1–1000)` → `result[]: { id, name, script, class, use_sqlite }`, `result_info: { count, page, per_page, total_count }` (verified live; e.g. `{ "id": "c375…", "name": "cendra_RateLimitDO", "script": "cendra", "class": "RateLimitDO", "use_sqlite": true }`). https://developers.cloudflare.com/api/resources/durable_objects/subresources/namespaces/methods/list/
- `GET …/namespaces/{id}/objects?limit=&cursor=` → `result[]: { id (64-hex), hasStoredData: bool }`, `result_info: { count, cursor }`; last page has `cursor: ""` (verified live). Docs say `limit` 10–10,000, but **live the server rejects anything below 100** (`limit=10/11/50/99` → HTTP 400 code 10077 "Malformed parameter: limit is too low"; `100`, `1000`, `10001` accepted). Use ≥100. https://developers.cloudflare.com/api/resources/durable_objects/subresources/namespaces/subresources/objects/methods/list/
- Undocumented but live: `GET …/namespaces/{id}` returns the single namespace object (verified live; wrangler `pages/download-config.ts` uses it). Not in the reference or SDK.

**Does not exist** (verified by absence in schema, SDK and wrangler): delete object, read/write an object's storage, list/delete alarms, evict/restart an object, delete a namespace. A namespace is deleted only by deploying the Worker with the class tombstoned (`[[migrations]] deleted_classes` / `exports` state `deleted`), which "removes its namespace and all of its stored data permanently" (https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/). Deleting the Worker is refused while other Workers bind the namespace (wrangler `delete.ts` reads `references.durable_objects[]`). Storage and alarms are reachable only from code inside the DO (`ctx.storage.deleteAll()`, `deleteAlarm()`), i.e. via a user-provided HTTP route.

Metrics (GraphQL; https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/): `durableObjectsInvocationsAdaptiveGroups { sum { requests responseBodySize } }`, `durableObjectsPeriodicGroups { sum { cpuTime } quantiles { memoryUsageBytesP50 … } }`, `durableObjectsStorageGroups { max { storedBytes } }` (verified live: query accepted, empty result for the probe window), `durableObjectsSubrequestsAdaptiveGroups`. The docs publish no field catalog ("Use GraphQL Introspection") — introspect `accountDurableObjectsPeriodicGroups` etc. at startup rather than hard-coding. Dimensions seen in third-party code: `namespaceId, objectId, coloCode, scriptName, datetime*`.

Dashboard: namespace list, Metrics tab (GraphQL, filterable by object id), Logs tab (Workers Logs, needs `observability.enabled`). No object browser, storage viewer or delete there either. **Verdict: PARTIAL — a read-only inventory (namespaces, object ids, `hasStoredData`, metrics). Deletion must be delegated to `wrangler deploy` with a tombstone; say so in the UI.**

---

## 6. Vectorize (v2)

Base `/accounts/{account_id}/vectorize/v2/indexes`. Docs https://developers.cloudflare.com/api/resources/vectorize/ ; wrangler `packages/wrangler/src/vectorize/client.ts` (paths match exactly). Permission: **Vectorize Read** for GET/query/get_by_ids/list, **Vectorize Write** for the rest. (The probe token lacks it: live `GET …/vectorize/v2/indexes` → HTTP 403 code 10000; the legacy v1 path `/vectorize/indexes` returns the same.)

| Op | Call | Notes |
|---|---|---|
| List | `GET …/indexes` | no pagination; `result[]` = index object |
| Create | `POST …/indexes` `{ name, description?, config: { dimensions (1–1536), metric: "cosine"\|"euclidean"\|"dot-product" } \| { preset } }` | presets: `@cf/baai/bge-small-en-v1.5`, `@cf/baai/bge-base-en-v1.5`, `@cf/baai/bge-large-en-v1.5`, `openai/text-embedding-ada-002`, `cohere/embed-multilingual-v2.0` |
| Get | `GET …/indexes/{name}` | `{ name, description?, created_on, modified_on, config: { dimensions, metric, preset? } }` |
| Delete | `DELETE …/indexes/{name}` | result is empty/null |
| **Update** | **none** | no PATCH/PUT; name/dimensions/metric/description immutable |
| Info | `GET …/indexes/{name}/info` → `{ dimensions, vectorCount, processedUpToDatetime, processedUpToMutation }` | compare `processedUpToMutation` with a returned `mutationId` to know whether a write has landed (writes are async) |
| Insert / Upsert | `POST …/insert` / `POST …/upsert`, `Content-Type: application/x-ndjson`, one `{ id (≤64 B), values: number[], namespace?, metadata? }` per line; `?unparsable-behavior=error\|discard` | → `{ mutationId }` only (no count/ids in v2); wrangler sends it as multipart `FormData`, both accepted |
| Query | `POST …/query` `{ vector, topK? (≤100; ≤50 with returnValues or returnMetadata:"all"), filter?, returnValues?, returnMetadata?: "none"\|"indexed"\|"all" }` → `{ count, matches: [{ id, score, values?, metadata?, namespace? }] }` | **undocumented but used by wrangler**: `namespace` in the body, and query-by-id = same path with `{ vectorId: "<id>", …options }` instead of `vector` (no separate route). Filter grammar: `{ field: v \| null \| { $eq,$ne,$in,$nin,$lt,$lte,$gt,$gte } }`, dotted paths |
| Get by ids | `POST …/get_by_ids` `{ ids }` → `result: [{ id, values, metadata?, namespace? }]` | |
| Delete by ids | `POST …/delete_by_ids` `{ ids }` → `{ mutationId }` | |
| **List vectors** | `GET …/indexes/{name}/list?count=(1–1000)&cursor=` → `{ count, totalCount, isTruncated, vectors: [{ id }], nextCursor, cursorExpirationTimestamp }` | ids only — pair with `get_by_ids`; snapshot semantics (new vectors not visible mid-pagination, cursors expire). https://developers.cloudflare.com/api/resources/vectorize/subresources/indexes/methods/list_vectors/ ; `wrangler vectorize list-vectors` |
| Metadata index | `POST …/metadata_index/create` `{ propertyName, indexType: "string"\|"number"\|"boolean" }` → `{ mutationId }`; `GET …/metadata_index/list` → `{ metadataIndexes: [{ propertyName, indexType }] }`; `POST …/metadata_index/delete` `{ propertyName }` → `{ mutationId }` | ≤10 per index |

Limits (https://developers.cloudflare.com/vectorize/platform/limits/): 50,000 indexes paid / 100 free; dims ≤1536; id ≤64 B; metadata ≤10 KiB/vector, indexed metadata ≤64 B; upsert batch ≤1000 via binding, **≤5000 via HTTP**; upload body ≤100 MB; namespaces ≤50,000 paid; vectors/index 20M on the limits page today (changelog 2026-01-23 said 10M — show the limits-page value).

Dashboard: list/create/delete/details + `info` counts only; no vector browser or query console. **Verdict: FULL — the TUI can exceed the dashboard (query, list-vectors, metadata indexes). Only gap: no rename/description edit.**

---

## 7. Hyperdrive

Base `/accounts/{account_id}/hyperdrive/configs`. Docs https://developers.cloudflare.com/api/resources/hyperdrive/ ; wrangler `packages/wrangler/src/hyperdrive/client.ts`. Permission: **Hyperdrive Read/Write** (probe token lacks it: live 403 code 10000).

| Op | Call |
|---|---|
| List | `GET …/configs?page=&per_page=(1–100)` |
| Create | `POST …/configs` |
| Get | `GET …/configs/{id}` |
| Replace / Edit | `PUT …/configs/{id}` (full) / `PATCH …/configs/{id}` (partial; the `origin` network block must always be sent whole) |
| Delete | `DELETE …/configs/{id}` |
| **Restart pool** | `POST …/configs/{id}/restart` (no body) → config with `restarted_on` — neither the dashboard nor wrangler exposes this. https://developers.cloudflare.com/api/resources/hyperdrive/subresources/configs/methods/restart/ |

Body:
```json
{ "name": "…",
  "origin": { "scheme": "postgres|postgresql|mysql", "host", "port", "database", "user", "password" }
           | { "scheme", "host", "database", "user", "password", "access_client_id", "access_client_secret" }   // Access tunnel, no port
           | { "scheme", "database", "user", "password", "service_id" },                                        // Workers VPC
  "caching": { "disabled": false, "max_age": 60 (1–3600), "stale_while_revalidate": 15 },
  "mtls": { "ca_certificate_id", "mtls_certificate_id", "sslmode": "require|verify-ca|verify-full" (PG) | "REQUIRED|VERIFY_CA|VERIFY_IDENTITY" (MySQL) },
  "origin_connection_limit": 5–20 free / 5–100 paid }
```
Response: `{ id (32 hex), name, origin (secrets omitted), caching, mtls?, origin_connection_limit, created_on, modified_on, restarted_on?, integration? }`. `password`/`access_client_secret` are write-only. No certificate endpoints under Hyperdrive — ids reference `/accounts/{id}/mtls_certificates`. Wrangler-only extra: `POST …/hyperdrive/integrationsOperations/{integration}/createDatabaseSignature` (experimental PlanetScale signature).

"Test connection": no endpoint, and none needed — create/update performs a background test connection (empty `;` query) and fails with API error codes **2008–2016** (bad hostname, non-public IP, cannot connect, refused, no TLS, bad credentials, no such database, generic, test query failed): https://developers.cloudflare.com/hyperdrive/observability/troubleshooting/ . Limits: 10 configs free / 25 paid; 15 s connect timeout; 60 s max statement; 50 MB cached response. GraphQL: `hyperdriveQueriesAdaptiveGroups` (dims `cacheStatus, eventStatus`; `count, queryBytes, resultBytes, connectionLatency, queryLatency`) and `hyperdrivePoolSizesAdaptiveGroups` (dim `coloCode`; `currentPoolSize, availablePoolSlots, waitingClients, maxPoolSize`), filter `configId`. https://developers.cloudflare.com/hyperdrive/observability/metrics/

**Verdict: FULL (plus `restart`, which the dashboard lacks).**

---

## 8. Workers Analytics Engine (brief)

Writes only from a Worker binding (`writeDataPoint`); datasets are created implicitly; no REST dataset CRUD exists for anyone. Reads:

- `POST /accounts/{account_id}/analytics_engine/sql` with the **raw SQL as the body** (no JSON wrapper); permission **Account Analytics Read**. Also accepts `GET …/sql?query=<urlencoded>` (verified live). https://developers.cloudflare.com/analytics/analytics-engine/sql-api/
- Not the v4 envelope — the body is whatever `FORMAT` produced. Verified live, `FORMAT JSON` (default):
  ```json
  { "meta": [ {"name":"dataset","type":"String"} ], "data": [ {"dataset":"originstat_events"}, … ], "rows": 3, "rows_before_limit_at_least": 7536 }
  ```
  `FORMAT JSONEachRow` → one object per line; `FORMAT TabSeparated` → TSV, no header (both verified). Errors are **plain text with HTTP 422**, e.g. `Input was invalid: cannot select unknown column: "nope"` (verified).
- Dataset discovery: **`SHOW TABLES`** (verified live → column `dataset`). Also `SHOW TIMEZONES`. Every dataset has `dataset, timestamp, _sample_interval, index1, blob1…blob20, double1…double20`; use `SUM(_sample_interval)` for counts. Single-table `SELECT` only (no JOIN/UNION); `LIMIT ALL` to lift the limit. No published query timeout/row cap — add a client timeout and a default `LIMIT`. https://developers.cloudflare.com/analytics/analytics-engine/sql-reference/statements/
- Newer general endpoint `POST /client/v4/analytics/sql` (`{ query, params, scope: { accountTag }, time_range }`; datasets addressed as `events.analyticsEngine.<name>`; `GET /client/v4/analytics/sql/introspection?account_tag=` lists datasets; returns `statistics`). https://developers.cloudflare.com/analytics/sql-api/query-api/

Dashboard shows only a per-binding data-point count; no query UI. **Verdict: FULL (read) — the TUI exceeds the dashboard.**

---

## 9. Token permissions the TUI should ask for (observed + documented)

| Product | Read | Write |
|---|---|---|
| D1 | D1 Read | D1 Write |
| KV | Workers KV Storage Read | Workers KV Storage Write |
| R2 (REST incl. objects) | Workers R2 Storage Read | Workers R2 Storage Write (+ Workers R2 Data Catalog R/W for `/r2-catalog`) |
| R2 (S3 / presign / >300 MB) | an R2 token: Access Key = token `id`, Secret = SHA-256(token `value`); minting one needs **API Tokens Write** on the caller |
| Queues | Queues Read | Queues Write (pull/ack too) |
| Durable Objects | Workers Scripts Read | — (nothing to write) |
| Vectorize | Vectorize Read | Vectorize Write |
| Hyperdrive | Hyperdrive Read | Hyperdrive Write |
| Analytics Engine / GraphQL | Account Analytics Read | — |

A missing permission is always HTTP 403 `{"code":10000,"message":"Authentication error","documentation_url":…}`; listing permission groups (`GET /accounts/{id}/tokens/permission_groups`) itself needs token-management rights (probe: 403 code 9109). Verify a token with `GET /user/tokens/verify` → `{ id, status: "active" }`.
