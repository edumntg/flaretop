# flaretop — plan

**Name:** `flaretop` (code name during research: stratus). Decided 2026-10-06.
**Date:** 2026-10-05
**Status:** Phases 1 and 2 implemented and live-tested on 2026-10-05 (see README for the feature table). Phase 3 not started.
**Author:** Eduardo Montilva (with Claude)

A fast, keyboard-driven terminal UI that replaces the Cloudflare web dashboard for
day-to-day work: Workers, D1, KV, R2, Queues, zones and DNS, and the rest of the
account. Single static binary, instant start, tiny memory footprint, works over SSH.

---

## 0. Why this exists when `cf` exists (decided 2026-10-05)

Cloudflare released `cf` on 2026-09-28: an open-beta CLI generated from the OpenAPI
spec with more than 2,900 commands, JSON output by default, built for agents, and
slated to replace wrangler. Measured here on beta 12 with Node 24: 378 MB on disk,
0.3 s and 85 MB for `--version`, 0.8–1.0 s and ~140 MB for any real command before
touching the network, no `workers tail`. Its `--json` output is the raw API envelope.

Consequences:
- **Not a CLI, a TUI.** "One command per operation" is solved by `cf`. This project is
  the interactive layer `cf` does not have: browse and edit data, paginate, live tail,
  versions and gradual deploys, DNS table, metrics. k9s to kubectl, lazygit to git.
- **No wrapper.** Spawning `cf` per action costs ~1 s and 140 MB each, and parsing its
  output is the same work as calling the API, so it buys only auth. We call the API
  directly from Rust.
- **Reuse `cf auth login` instead of registering our own OAuth client.** `cf` stores
  `oauth_token`, `refresh_token`, `expiration_time`, `scopes` (and optionally `api_token`)
  as plain JSON under its config dir, with all ~476 scopes. We read it opt-in, never
  refresh with its refresh token (we tell the user to run `cf auth login` again), and
  keep `CLOUDFLARE_API_TOKEN` / pasted token as the universal path. Phase 4's own OAuth
  client is dropped unless the `cf` route proves insufficient.
- **"Copy as `cf` command"** on every action: cheap, useful for agents, the honest
  bridge between the two tools.
- **v1 scope = Phases 1 and 2 only** (Workers platform: data, logs, deploys). Zones and
  DNS come after there is traction.

## 1. What the research established

Four reports under `docs/research/` back every claim below. Short version:

**Everything the dashboard does for the products we care about is reachable without a
browser.** The public REST API has 3,647 operations across 2,287 paths; charts come from
the GraphQL Analytics API; live logs and Workflow instance streams are two WebSockets.
Only a handful of actions are genuinely dashboard-only, and none of them is daily work.

| Area | Verdict | Notes |
|---|---|---|
| Workers (list, download bundle, settings, upload, secrets, vars, cron, workers.dev, custom domains, routes, versions, deployments, gradual rollout, rollback) | FULL | Secret values are never readable (by design). Rollback = new deployment at 100%. |
| Workers live logs (`wrangler tail`) | FULL | WebSocket, sub-protocol `trace-v1`, ping every 10 s, JSON frames. Implementable in Rust. |
| Workers Logs (persistent, Query Builder) | FULL | `telemetry/query|keys|values` + live-tail. Needs Workers Observability permission. |
| D1 | FULL | `/query` + `/raw`, export/import (async), Time Travel. No table-list endpoint: query `sqlite_master`. |
| KV | FULL | Keys cursor-paginated (limit 10–1000), bulk get max 100. No exact key count anywhere. |
| R2 | FULL for buckets and objects ≤ 300 MB with the plain account token | Objects list/get/put/delete are public REST now. Multipart, presign, HEAD and Range need the S3 API with an R2 key the tool can mint itself. |
| Queues | PARTIAL | Full management, push, pull/ack, purge, metrics. The dashboard's non-consuming message browser is not public. |
| Durable Objects | PARTIAL | Namespaces, object ids, `hasStoredData`, SQL query on SQLite-backed DOs, metrics. No storage read, delete or alarms. |
| Workflows | FULL | Instances, steps, pause/resume/terminate/restart, send event, live subscribe. |
| Vectorize, Hyperdrive | FULL | Hyperdrive even has a pool `restart` the dashboard lacks. |
| Pages | PARTIAL | Everything incl. Direct Upload and build logs. Git-app install is dashboard-only. Product is in maintenance; Workers Assets is the future. |
| Zones, DNS, SSL, cache, rulesets, WAF, Access, Tunnels, Turnstile | FULL | DNS batch endpoint, BIND import/export, DNSSEC. Zone settings batch endpoint is deprecated (EOL 2027-03-31): use per-setting calls. |
| Registrar | PARTIAL | New API (beta, 2026-09) lists, searches and registers domains. Renewals, contacts and transfer-lock are still dashboard-only. Legacy `/registrar/domains` is past EOL. |
| Account: members, tokens, audit logs, notifications | FULL | Tokens can be created and rolled via API. Audit log v2 is cursor-paginated. |
| Billing | PARTIAL | History, credits, subscriptions readable. Adding a card needs a browser (Stripe intent). No per-product "this month" REST: use GraphQL. |
| Workers AI, AI Gateway, Browser Rendering, Images, Stream, Email Routing | FULL | Lower priority. |
| Containers | PARTIAL | List/scale/rollout/delete via API; image build+push needs local Docker. |

**Authentication.** Three options, all viable:
1. **API token** (`CLOUDFLARE_API_TOKEN` or pasted once). Universal. Verify with `GET /user/tokens/verify`. Missing permissions show up as HTTP 403 with code `10000` or `9109`; the tool must map those to "this token lacks X" screens.
2. **OAuth login** like `wrangler login`. Cloudflare now has self-serve OAuth client registration (dashboard → Manage Account → OAuth clients, or `POST /accounts/{id}/oauth_clients`). Authorization-code + PKCE S256, public client, localhost redirect, `offline_access` refresh tokens. Scopes map 1:1 to API token permission names (`GET /oauth/scopes`). **Catch:** a new client is *private* (only members of the registering account can authorize it). Making it public is permanent and requires a client URL, logo and DNS TXT verification of a domain. So OAuth for other users needs the product name and domain decided first.
3. **Reuse wrangler's token** (opt-in): `~/.config/.wrangler/config/default.toml` holds `oauth_token`, `refresh_token`, `expiration_time`, `scopes`. Convenient for people who already use wrangler, but wrangler's scope set excludes DNS, SSL, billing, tokens, so it only unlocks the Workers-platform half.

Borrowing wrangler's public `client_id` is technically possible and will not be done: the consent screen would say "Wrangler".

**Rate limit.** 1,200 requests per 5 minutes per user, shared with the dashboard and every token. A TUI that fans out on every keystroke will hit it. Caching, coalescing and lazy loading are architectural requirements, not polish. GraphQL is separately limited to 300 queries per 5 minutes.

**Pagination is not uniform.** Four families: `page`/`per_page` with `result_info`; `page`/`per_page` with no `result_info` (Workers scripts, secrets, tails); cursor (`result_info.cursor`, empty string = end; KV keys, R2 objects, DO objects, audit log v2, Workflow instances); `page_token` (Containers). Some lists wrap the array (`result.items`, `result.deployments`, `result.schedules`, `result.buckets`).

**Hard gaps to state honestly in the UI** (not fixable client-side):
- Queues: no non-consuming peek; "preview" via pull+retry costs an attempt and only works with an `http_pull` consumer.
- Durable Objects: inventory only; deleting a namespace needs a `wrangler deploy` with a tombstone migration.
- Secrets: names only, never values.
- R2 uploads over 300 MB: S3 path with a minted R2 token.
- Git integration install (Workers Builds, Pages), adding a payment card, registrar renewals/contacts, Containers image build.

---

## 2. Technology decision: Rust

Go was considered. Rust wins for the same reasons it did on lazymongo and lazyreq, plus two new ones:

| | Rust | Go |
|---|---|---|
| Binary (release, stripped, static musl) | ~4–7 MB with reqwest+rustls | ~15–25 MB |
| Idle RSS | ~5–10 MB, no GC | ~15–30 MB |
| Startup | instant | instant |
| TUI | ratatui 0.30 + crossterm 0.29 (gitui, yazi, atuin, bottom) | Bubble Tea (excellent) |
| HTTP/WS/TLS without OpenSSL | reqwest 0.13 + rustls, tokio-tungstenite | stdlib (fine) |
| Reuse | lazymongo's actor model, theme engine, palette, modal/textarea, release pipeline, smoke tests | start from zero |
| Official Cloudflare SDK | none in either language worth using (see below) | cloudflare-go exists, but is generated and heavy |

The `cloudflare` crate (cloudflare-rs 0.14, last release 2025-03, pinned to reqwest 0.12, no D1/Queues/versions/R2 objects/GraphQL) is not used. The client is a thin hand-written layer over reqwest: one `Endpoint` trait, one envelope struct, one pagination helper, typed request/response structs lifted from the OpenAPI spec as needed.

### Stack (pinned at project start, 2026-10-05)

| Need | Crate | Why |
|---|---|---|
| TUI | `ratatui` 0.30.2, `crossterm` 0.29 | current; lazymongo is on 0.29 and the port notes are in §5 |
| Text editing | `ratatui-textarea` 0.9 (or port lazymongo's own `textarea.rs`) | `tui-textarea` 0.7 does not build with ratatui 0.30 |
| Async | `tokio` current_thread runtime | same as lazymongo; one actor task, UI never blocks |
| HTTP | `reqwest` 0.13, `default-features = false`, `rustls-no-provider`, `http2`, `gzip`, `brotli`, `json`, `multipart`, `stream` + `rustls` 0.23 with `ring` | 0.13 renamed the TLS features (`rustls-tls` is gone) and defaults to aws-lc-rs; we pin ring and call `install_default()` at startup. Multipart for script upload. |
| WebSocket | `tokio-tungstenite` 0.30 with `rustls-tls-webpki-roots` | tail, Workflows subscribe; reuses ring, +150 KB |
| S3 (R2 large objects, presign) | `rusty-s3` | pure SigV4 signer, no AWS SDK, no runtime |
| JSON | `serde`, `serde_json` (`preserve_order`), `json5` for editors | |
| Config | `toml`, `dirs` | `~/.config/<name>/config.toml`, token file mode 0600 |
| OAuth | hand-rolled: PKCE via `sha2` + `base64`, loopback via `tokio::net::TcpListener`, browser via `open` | the `oauth2` crate adds more than it saves |
| Hashing | `sha2` (S3 secret = SHA-256 of token), `blake3` only if Pages direct upload ships | |
| Clipboard | crossterm OSC 52 + `arboard` fallback (copy lazymongo's approach) | |
| Charts | ratatui built-ins (`Chart`, `Sparkline`, `BarChart`) | metrics panes |

TLS backend: `rustls` with `ring`. It builds on every target in the release matrix (x86_64/aarch64 musl, both macOS, Windows MSVC) with only `musl-tools`; `aws-lc-rs` adds ~0.6–4 MB and needs cmake/nasm on some runners. rustls panics on first use if two providers are in the graph, so CI must refuse `aws-lc-rs`:

```sh
cargo tree -e features --target "$TARGET" -i aws-lc-rs && exit 1
```

Measured on 2026-10-05 (x86_64 linux, rustc 1.97, release profile below): hello world 0.29 MB; reqwest+ring+http2 2.27 MB; + tokio-tungstenite 2.42 MB; + rusty-s3 2.43 MB. The AWS S3 SDK alone would be 5.5 MB and a 6-minute clean build, which is why `rusty-s3` wins.

Release profile: `lto = true`, `codegen-units = 1`, `opt-level = "z"`, `strip`, `panic = "abort"` (copied from lazymongo). Budget: binary ≤ 8 MB, idle RSS ≤ 15 MB, cold start ≤ 100 ms.

---

## 3. Architecture

Two crates, same split as lazymongo:

```
flaretop-core/        no TUI dependency; unit-testable against recorded JSON
  api/
    client.rs        reqwest wrapper: base URL, auth header, envelope, errors (code 10000/9109 → MissingPermission),
                     429 → backoff with Retry-After, request budget (token bucket under 1,200/5 min)
    paginate.rs      the four pagination families behind one async stream
    graphql.rs       POST /graphql, typed per dataset
    cache.rs         per-session memo (TTL) so re-entering a pane does not re-fetch
  auth/
    token.rs         CLOUDFLARE_API_TOKEN, config file, verify, permission probe
    oauth.rs         PKCE flow, loopback listener, refresh, revoke
    wrangler.rs      optional import of ~/.config/.wrangler/config/default.toml
    store.rs         0600 credential file; keyring later if demanded
  products/
    workers.rs  d1.rs  kv.rs  r2.rs  r2s3.rs  queues.rs  durable_objects.rs
    workflows.rs  vectorize.rs  hyperdrive.rs  pages.rs  zones.rs  dns.rs
    ssl.rs  cache.rs  rulesets.rs  registrar.rs  access.rs  tunnels.rs
    tokens.rs  members.rs  audit.rs  alerts.rs  billing.rs  ai.rs
  tail.rs            WebSocket tail client (trace-v1, ping/pong, reconnect, filters)
  actor.rs           Command → CoreEvent over mpsc; generation tags cancel stale work (same shape as lazymongo)
  types.rs           Command, CoreEvent, domain structs

flaretop/             the binary
  main.rs            arg parsing (no clap: hand-rolled like lazymongo), term init, panic hook
  app.rs             App state, key routing, screens
  ui.rs              layout: sidebar (products tree) | main pane | detail pane; status bar; help bar
  screens/           one module per product screen (list + detail + actions)
  modal.rs  palette.rs  textarea.rs  theme.rs  json_view.rs  table.rs  (ported from lazymongo)
  update.rs          self-update (ported)
```

Rules carried over from lazymongo:
- All I/O in the actor; the render loop never awaits the network.
- Every destructive action is confirmed; bulk deletes require typing a count or name; `--readonly` blocks writes at the I/O layer.
- Credentials never on screen; token file 0600; nothing from the API is logged to disk.
- Stale results dropped by generation token; `Esc` cancels.
- Every action reachable by mouse, keys, and the fuzzy command palette.

Navigation model: a left tree of **Account → product → resource** (like the dashboard's sidebar), a main pane that lists, and a detail pane. `Ctrl-T` fuzzy-jumps to any resource by name (any product). `:` opens the palette.

---

## 4. Phases

Each phase ends with a tagged release that is useful on its own. Order follows daily
frequency of use, not API difficulty.

### Phase 0 — Research and decisions  ✅ done (2026-10-05)
- API coverage verified per product against a real account (`docs/research/`).
- Language and stack decided (§2).
- Open decisions parked (§6).

### Phase 1 — Skeleton, auth, and read-only Workers platform  ✅ done (2026-10-05)
Goal: open the tool, pick an account, browse everything the Workers platform has, read-only.
- Workspace, CI (fmt, clippy `-D warnings`, tests), release workflow for 5 targets (copied from lazymongo).
- `api::client` with envelope, error mapping, retry/backoff, request budget, the four paginators. Unit tests with recorded JSON fixtures (shapes from the research).
- Auth: API token (env → config → prompt), verify, account picker (`/accounts`, with `/memberships` when the token is user-scoped). Permission probe that lights up which products the token can see; 403s render as "token lacks X" not as errors.
- Screens (read-only): Workers list + detail (settings, bindings, cron, domains, versions, deployments), D1 databases + tables + `SELECT` console with `/raw`, KV namespaces + keys + value/metadata viewer, R2 buckets + object browser (prefix/delimiter folders) + download, Queues list + consumers, Durable Objects namespaces + object ids.
- UI chrome ported from lazymongo: themes, palette, modal, help, status bar, mouse.
- Acceptance: cold start < 100 ms, idle RSS < 15 MB, browsing a 100k-key namespace or a 50k-object bucket keeps memory flat (cursor paging), no request storm (budget counter visible in status bar in debug).

### Phase 2 — Writes, logs and deploys (daily driver)  ✅ done (2026-10-05), minus D1 export/import and R2 S3 path
Goal: stop opening the dashboard for the Workers platform.
- D1: insert/edit/delete rows with field-level diff, DDL via console, export (async polling → file), import (init/upload/ingest/poll), Time Travel bookmark + restore (confirmed, shows `previous_bookmark` for undo).
- KV: put (value + metadata + TTL), delete, bulk delete by prefix (dry-run count first), bulk get for the table view.
- R2: upload ≤ 300 MB via REST, delete, bulk delete, CORS/lifecycle/public-access (`r2.dev`) toggles, custom domains. Large upload path: mint an R2 token (needs API Tokens Write), derive S3 keys, multipart via `rusty-s3`; presigned URL generator.
- Queues: create/delete/pause/purge, push message, consumer settings, pull/ack console for `http_pull` queues with the "this consumes an attempt" warning. Backlog from GraphQL.
- Workers: secrets put/delete (bulk endpoint), plain vars edit (settings PATCH with `keep_bindings`), cron edit, workers.dev toggle, custom domain attach, routes per zone, delete script. Versions: list, deploy a version at N% (gradual), rollback (= deploy old version at 100%).
- **Live tail**: WebSocket client (sub-protocol `trace-v1`, send `{"debug":false}` on open or nothing arrives, ping every 10 s, re-create before `expires_at`, `DELETE` the tail on exit) with filters (status, method, header, search, sampling), reconnect, `overload` notices, JSON/pretty views, pause/follow, save to file.
- Workflows: instances list with status filter, instance detail with steps and attempts, pause/resume/terminate/restart, trigger with params, send event.
- `--readonly` flag and session operations log (`L`).
- Acceptance: a full day of work on a real project (socialear or originstat) without the dashboard.

### Phase 3 — Zones, DNS, domains, account
Goal: replace the dashboard for the "website" half.
- Zones list + overview (status, plan, nameservers, dev mode), zone settings editor (per-setting endpoints; SSL mode, Always HTTPS, min TLS, cache level, security level, Under Attack toggle).
- DNS: records table with filters/search, create/edit/delete, batch apply, proxied toggle, BIND export/import, DNSSEC status.
- Cache purge: everything, by URL, by tag/host/prefix (all modes on every plan now; Free is limited to 5 purge requests per minute, so queue them).
- SSL: certificate packs, universal SSL status, Origin CA issue.
- Registrar: list registered domains with expiry/auto-renew (new API), auto-renew toggle, availability search. Renewals stay a "open in dashboard" link.
- Rulesets: list rules per phase (redirects, cache, WAF custom, rate limiting) with enable/disable; editing expressions in a later phase.
- Account: API tokens list/create (permission-group picker)/roll/delete, members list/invite, audit log v2 with filters, notification policies + webhooks, billing history + subscriptions (read).
- Access apps + policies and Tunnels list/status (read-only), Turnstile widgets.
- Acceptance: adding a site, pointing DNS, flipping SSL to Full (strict), purging cache, rotating a token, all from the terminal.

### Phase 4 — OAuth login, metrics, and the long tail
- **OAuth login**: register the OAuth client under the chosen product name (private first, public after domain verification), implement PKCE + loopback + refresh + revoke, `login`/`logout`/`whoami` commands. Optional wrangler token import. Token rotation handled transparently in the client.
- **Metrics** panes via GraphQL: Workers invocations/errors/CPU p50-p99, D1 reads/writes/size, R2 storage/ops, KV ops, Queues backlog/lag, DO requests/CPU, zone HTTP requests/bandwidth/cache ratio. Sparklines in list rows, charts in detail. Month-to-date usage estimate per product (the dashboard's "billable usage" page).
- Workers Logs (persistent): query builder (filters, needle, group-by), keys/values autocomplete, live-tail v2.
- Vectorize (indexes, info, query console, list vectors), Hyperdrive (configs, restart), Pages (projects, deployments, logs, rollback, env vars), Workers AI (model search, run console with SSE), AI Gateway (gateways, logs), Email Routing (rules, addresses), Containers (apps, instances, rollout).
- Script upload: deploy a local bundle (pre-built by the user's toolchain) as a version; assets upload session for static sites.

### Phase 5 — Polish, name, release
- Decide the name and domain; rename code name; OAuth client public verification.
- Self-update, Homebrew tap, `curl | sh` installer, Windows build, docs site.
- Smoke tests with `expect` scripts (lazymongo's `scripts/smoke-*.exp` pattern) against recorded fixtures and a sandbox account.
- Performance pass: binary size, RSS, request counts per screen, startup.
- Public repo + launch.

---

## 5. Porting notes from lazymongo (ratatui 0.29 → 0.30)

Known breakages when lifting `theme.rs`, `modal.rs`, `ui.rs`, `json_view.rs`, `textarea.rs`:
1. MSRV 1.88, edition 2024.
2. crossterm 0.29; `Color`/`Style` conversions use `FromCrossterm`/`IntoCrossterm`, not `From`/`Into`.
3. `Block` titles: `block::Title` and `Position` removed; `Block::title(impl Into<Line>)`, `.title_bottom(..)`.
4. `Alignment` → `HorizontalAlignment` (alias kept).
5. `Style` no longer implements `Styled`; `Stylize` methods are inherent on `Style`.
6. `Backend` has an associated `Error`; generic `fn run<B: Backend>` signatures change (`CrosstermBackend::Error = io::Error`).
7. `Flex::SpaceAround` semantics changed; `Marker` is `#[non_exhaustive]`.
8. `WidgetRef` blanket impl reversed: implement `Widget for &MyWidget`.
9. `List::highlight_symbol` takes `Into<Line>`.
10. `tui-textarea` 0.7 does not build; use `ratatui-textarea` 0.9 or the in-house textarea.

Unchanged: `Frame::render_widget`, `Terminal::draw`, `Layout::split`/`areas`, `ListState`/`TableState`, `ratatui::init()/restore()`.

---

## 6. Open decisions (do not block Phases 1–3)

1. **Name and domain.** Needed before the OAuth client can go public (Phase 4). Not `lazy-*`.
2. **OAuth client ownership.** Register under Eduardo's account; the client is private until domain-verified. Until then, API token is the documented path for everyone else.
3. **Token storage.** Plain file 0600 like wrangler (works headless, over SSH) vs OS keyring (`keyring` crate; flaky on headless Linux). Default: file; keyring as opt-in later.
4. **R2 large uploads.** Mint an R2 token automatically (needs API Tokens Write on the user's token) vs ask the user to paste S3 keys. Default: offer both.
5. **Scope line.** Zero Trust beyond Access apps/Tunnels, Magic networking, Radar, Email Security, Stream, Images, Load Balancers are out until someone asks.
6. **Pages.** Ship read + deployments + rollback; Direct Upload only if a user needs it (Workers Assets is the future).
7. **License.** MIT like lazymongo, unless the product strategy changes.

---

## 7. References

- `docs/research/api-inventory.md` — per-area operation counts from the OpenAPI spec and the gaps section.
- `docs/research/storage-apis.md` — D1, KV, R2, Queues, DO, Vectorize, Hyperdrive, Analytics Engine, verified live.
- `docs/research/compute-apis.md` — Workers, tail protocol, Workers Logs, Pages, Workflows, AI, Containers, account-level.
- `docs/research/zones-and-rust.md` — zones, DNS, registrar, SSL, cache, rulesets, security, Zero Trust, analytics, account/billing; Rust crate audit and ratatui 0.30 breaking changes.
- Auth: https://developers.cloudflare.com/fundamentals/oauth/create-an-oauth-client/ , wrangler's `packages/workers-auth/src/` in cloudflare/workers-sdk.
- Limits: https://developers.cloudflare.com/fundamentals/api/reference/limits/
