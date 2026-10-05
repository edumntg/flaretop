# Cloudflare zone/edge APIs + Rust ecosystem check for a dashboard-replacement TUI

Date: 2026-10-05. Method for Part 1: read-only `GET`s against `https://api.cloudflare.com/client/v4` with the token in `/root/.cf-env` (ids redacted below), plus the official OpenAPI document `https://raw.githubusercontent.com/cloudflare/api-schemas/main/openapi.json` (2,287 paths, downloaded today) for exact paths, query params, enums and deprecation flags, plus the deprecations page `https://developers.cloudflare.com/fundamentals/api/reference/deprecations/`. Nothing was written, created or changed.

## What the read-only token in /root/.cf-env can and cannot see (useful for the TUI's "permission missing" UX)

`GET /user/tokens/verify` → `{"status":"active"}`. Observed scopes:

| Works (200) | Fails (403) |
|---|---|
| `/zones` (list, 6 zones), `/zones/{id}`, `/zones/{id}/holds` | `/zones/{id}/settings*` (needs **Zone Settings Read**) |
| `/accounts/{id}` , `/members`, `/roles`, `/audit_logs`, `/logs/audit` (v2) | `/zones/{id}/dns_records*`, `/dnssec`, `/dns_settings` (**DNS Read**) |
| `/accounts/{id}/access/apps`, `/access/policies`, `/access/service_tokens`, `/access/identity_providers` | `/zones/{id}/ssl/*`, `/custom_hostnames`, `/custom_certificates`, `/settings/min_tls_version` (**SSL and Certificates Read**) |
| `/accounts/{id}/cfd_tunnel`, `/tunnels` | `/zones/{id}/cache/*`, `/argo/*`, `/purge_cache` (**Cache Purge / Cache Settings**) |
| `/accounts/{id}/challenges/widgets` (Turnstile) | `/zones/{id}/rulesets*`, `/pagerules`, `/firewall/*`, `/bot_management` (**Zone WAF / Firewall Services / Page Rules / Bot Management**) |
| `/accounts/{id}/alerting/v3/*` (policies, webhooks, history, available_alerts) | `/accounts/{id}/registrar/*` (**Registrar**) |
| `/accounts/{id}/rum/site_info/list` (Web Analytics) | `/accounts/{id}/load_balancers/*`, `/zones/{id}/healthchecks`, `/spectrum/apps`, `/waiting_rooms` |
| `/certificates?zone_id=` (Origin CA list) | `/accounts/{id}/billing/*`, `/subscriptions`, `/zones/{id}/subscription`, `/available_plans` (**Billing**) |
| `/accounts/{id}/dns_settings` (account defaults) | `/user`, `/memberships`, `/user/tokens`, `/accounts/{id}/tokens`, `/tokens/permission_groups` (**API Tokens Read / User Details**) |
| `/accounts/{id}/workers/scripts`, `/d1/database`, `/r2/buckets`, `/queues`, `/storage/kv/namespaces` | `/accounts/{id}/devices*`, `/gateway/rules`, `/access/groups`; `/pages/projects`; GraphQL zone analytics (`authz` error names the missing permission `com.cloudflare.api.account.zone.analytics.read`) |

Two different 403 shapes mean "missing scope": `{"code":10000,"message":"Authentication error"}` and `{"code":9109,"message":"Unauthorized to access requested resource"}`. A `401 {"code":1001,"Not authorized"}` came back for a tunnel id that does not exist. `400 {"code":7000,"No route for that URI"}` means the path is wrong (e.g. `/user/invoices`), and `400 {"code":7001,"Method GET not available"}` is the answer for `GET /zones/{id}/activation_check` (it is PUT-only). Map all of these to distinct TUI messages.

Pagination conventions seen in the wild: classic `result_info: {page, per_page, total_pages, count, total_count}` (zones, members, roles, Access, Turnstile, RUM); **cursor** pagination `result_info: {count, cursor}` for the new audit-log v2 (`/accounts/{id}/logs/audit`) and the new Registrar endpoints; no `result_info` at all for some lists (alerting policies, Workers scripts, R2 buckets, Origin CA `/certificates` returns the classic shape with `per_page: 20`).

---

## PART 1 — Zone / domain / edge APIs

### 1. Zones — verdict: FULL

Confirmed live (`GET /zones?account.id=…&status=active&per_page=50`): `result_info {page:1, per_page:50, total_pages:1, count:6, total_count:6}`; zone object keys: `account, activated_on, created_on, development_mode, id, meta, modified_on, name, name_servers, original_dnshost, original_name_servers, original_registrar, owner, paused, permissions, plan, status, tenant, tenant_unit, type, vanity_name_servers, vanity_name_servers_ips`. `plan.name` was `"Free"`, `type` `"full"`, `paused` `false`.

| Operation | Endpoint | Notes (from OpenAPI) |
|---|---|---|
| List | `GET /zones` | query: `name`, `status` enum `initializing|pending|active|moved`, `type[]`, `account.id`, `account.name`, `page` (≥1), **`per_page` 5–50, default 20**, `order` enum `name|status|account.id|account.name|plan.id`, `direction`, `match` `any|all` |
| Get | `GET /zones/{zone_id}` | |
| Create | `POST /zones` body `{account:{id}, name, type}` — `type` enum `full|partial|secondary|internal` (default `full`) | `name` max 253 chars, regex-validated |
| Delete | `DELETE /zones/{zone_id}` | |
| Pause | `PATCH /zones/{zone_id}` body `{paused: true}` ("zone is only using Cloudflare DNS services") | same PATCH also accepts `type`, `vanity_name_servers`; `plan` there is **deprecated** in favour of the subscription endpoint |
| Activation check | `PUT /zones/{zone_id}/activation_check` — "can be triggered every 5 min for paygo/ent customers, every hour for FREE zones" | GET returns 7001 |
| Holds | `GET/POST/DELETE/PATCH /zones/{zone_id}/holds` — live result `{hold, hold_after, include_subdomains}` | |
| Settings, one call | `GET /zones/{zone_id}/settings` and `PATCH /zones/{zone_id}/settings` (bulk `items`) — **both flagged `deprecated: true` in the spec; deprecation 2025-04-23, end of life 2027-03-31** ("The Zone Settings Batch API endpoints … will reach their end of life on March 31, 2027") | Still usable for the next 18 months; plan to replace with N parallel per-setting GETs |
| Setting, per id | `GET/PATCH /zones/{zone_id}/settings/{setting_id}` body `{value}` (or `{enabled}` for ssl-recommender) | Permission: **Zone Settings Write** |
| Plan / subscription | `GET/POST/PUT/DELETE /zones/{zone_id}/subscription` body `{rate_plan:{id:"free"…}, frequency: weekly|monthly|quarterly|yearly, …}`; `GET /zones/{zone_id}/available_plans` | Changing plan = creating a subscription → billing profile with payment method required |

Setting values confirmed from the spec (`components.schemas.zones_*`): `ssl` = `off|flexible|full|strict|origin_pull`; `always_use_https` = `on|off`; `minify` = `{css,html,js: on|off}` (**deprecated 2024-08-05**, the dashboard no longer shows it); `brotli` = `on|off`; `development_mode` = `on|off` (3-hour auto-off; GET returns `time_remaining`); `cache_level` = `bypass|basic|simplified|aggressive|cache_everything` (zone-level values are `basic|simplified|aggressive`, the other two are Page Rule values); `browser_cache_ttl` = integer 0–31536000; `security_level` = `off|essentially_off|low|medium|high|under_attack` (**"I'm Under Attack" mode is just `security_level: under_attack`**); `min_tls_version` = `1.0|1.1|1.2|1.3`; `tls_1_3` = `on|off|zrt`; `http3`, `websockets`, `0rtt`, `early_hints`, `automatic_https_rewrites`, `opportunistic_encryption`, `rocket_loader`, `email_obfuscation`, `browser_check`, `ip_geolocation`, `polish` (`off|lossless|lossy`), `mirage`, `ciphers` (array), `cname_flattening` (`flatten_at_root|flatten_all`, **deprecated 2025-06-08 → use `/zones/{id}/dns_settings.flatten_all_cnames`**). Full id table with values: https://developers.cloudflare.com/api/resources/zones/subresources/settings/methods/edit/ . Each response carries `editable: bool` — render read-only when false (plan-gated settings such as `advanced_ddos`).

Dashboard-only: the zone "Quick Start Guide" wizard and the "Overview" cards are compositions of the above; moving a zone between accounts has no API (delete + re-create); plan upgrades that need a new payment method (the API can only create a Stripe payment intent, see §11).

### 2. DNS records — verdict: FULL

| Operation | Endpoint | Notes |
|---|---|---|
| List | `GET /zones/{zone_id}/dns_records` | filters: `name`, `name.exact/contains/startswith/endswith`, `type` (enum of 21 types below), `content` + same suffixes, `proxied`, `comment` (+ `.present/.absent/.exact/...`), `tag` (+ suffixes), `tag_match`, `match any|all`, free-text `search`; sort `order` enum `type|name|content|ttl|proxied` + `direction`; `page` ≥1; **`per_page` min 1, max 5,000,000, default 100** (so yes, one page per zone is fine); `include_shadow_metadata`, `shadowed_by_name`, `shadowing_name` (Internal DNS) |
| Create / overwrite / patch / delete | `POST /zones/{zone_id}/dns_records`, `PUT|PATCH|DELETE /zones/{zone_id}/dns_records/{dns_record_id}` | A/AAAA/CNAME carry `proxied`; `ttl: 1` = auto; `comment`, `tags[]`; CNAME has `settings.flatten_cname` (per-record flattening, "unavailable for proxied records, since they are always flattened") |
| Batch | `POST /zones/{zone_id}/dns_records/batch` body `{deletes:[{id}], patches:[...], puts:[...], posts:[...]}` | executed in that order; "single database transaction" but KV propagation "is not atomic"; "If any of the individual action fails, no changes are applied and the API returns the first error"; **Free: 200 records per batch, Pro/Business/Enterprise: 3,500** — https://developers.cloudflare.com/dns/manage-dns-records/how-to/batch-record-changes/ |
| Export BIND | `GET /zones/{zone_id}/dns_records/export` (text/plain zone file) | |
| Import BIND | `POST /zones/{zone_id}/dns_records/import` multipart `file` (+ `proxied`) | |
| Scan | `POST /dns_records/scan/trigger`, `GET/POST /dns_records/scan/review` (`accepts`, `rejects`) — the old `POST /dns_records/scan` is deprecated | |
| Usage | `GET /zones/{zone_id}/dns_records/usage` | record count vs. quota |
| DNSSEC | `GET/PATCH/DELETE /zones/{zone_id}/dnssec` — PATCH body `{status: active|disabled, dnssec_multi_signer, dnssec_presigned, dnssec_use_nsec3}`; GET returns `algorithm, digest, digest_algorithm, digest_type, ds, flags, key_tag, key_type, public_key, status, modified_on` (what you paste at the registrar) | |
| Zone DNS settings | `GET/PATCH /zones/{zone_id}/dns_settings` — keys `flatten_all_cnames, foundation_dns, internal_dns, multi_provider, ns_ttl, secondary_overrides, soa{...}, zone_mode, nameservers{type}` | **CNAME flattening lives here now** |
| Account defaults | `GET/PATCH /accounts/{id}/dns_settings` — live result: `{"zone_defaults":{"nameservers":{"type":"cloudflare.standard"},"foundation_dns":false,"multi_provider":false,"secondary_overrides":false,"soa":{...},"ns_ttl":86400,"zone_mode":"standard","internal_dns":{...},"flatten_all_cnames":false}}` | |

Record types in the spec (one schema each): `A, AAAA, CAA, CERT, CNAME, DNSKEY, DS, HTTPS, LOC, MX, NAPTR, NS, OPENPGPKEY, PTR, SMIMEA, SRV, SSHFP, SVCB, TLSA, TXT, URI` (21). Structured types use a `data{}` object (e.g. SRV `priority/weight/port/target`, CAA `flags/tag/value`).

Dashboard-only: nothing material. (The dashboard's "Import and Export" page just calls the two endpoints above; "DNS record quick scan" is the scan endpoints.)

### 3. Registrar — verdict: PARTIAL, and the premise "no purchase via API" is now wrong

As of a **beta announced with docs last updated 2026-09-16** (https://developers.cloudflare.com/registrar/registrar-api/) Cloudflare has a registration API:

| Operation | Endpoint |
|---|---|
| Search | `GET /accounts/{id}/registrar/domain-search?q=&extensions[]=&limit≤50` |
| Availability + price | `POST /accounts/{id}/registrar/domain-check` body `{domains:[…]}` (≤20 per request) |
| TLD rules | `GET /accounts/{id}/registrar/extensions` (cursor, `per_page` ≤50), `GET …/extensions/{extension}` (returns `registration_schema` incl. required `contact_extensions` like `.us` nexus) |
| **Register (purchase)** | `POST /accounts/{id}/registrar/registrations` body `{domain_name, years, auto_renew, privacy_mode, contacts, contact_extensions, acknowledgements}` — "This is a billable operation — successful registration charges the account's default payment method. All successful domain registrations are non-refundable"; prerequisites: billing profile with default payment method, default registrant contact, ≤500 domains per account |
| List / get | `GET /accounts/{id}/registrar/registrations` (cursor, `sort_by registry_created_at|registry_expires_at|name`), `GET …/registrations/{domain_name}` → `{auto_renew, created_at, domain_name, expires_at, locked, privacy_mode: off|redaction, status: active|registration_pending|transfer_pending|…}` |
| Update | `PATCH …/registrations/{domain_name}` body **`{auto_renew}` only** |
| Status polling | `GET …/registrations/{domain_name}/registration-status`, `…/update-status`, `…/transfer-in-status` |
| Transfer in | `POST …/registrations/{domain_name}/transfer-in` body `{auth_code, auto_renew, contacts, contact_extensions, privacy_mode}` and `POST /registrar/domain-transfer-check` are in the OpenAPI spec, but the docs page still says "Transfers are not yet available through the API" — treat as rolling out |
| Sandbox | every endpoint also exists under `/accounts/{id}/registrar-sandbox/…` for testing without charges |

Legacy endpoints `GET /accounts/{id}/registrar/domains`, `GET/PUT …/domains/{domain_name}` (PUT body `{auto_renew, locked, privacy}`) are flagged deprecated in the spec: "deprecated and will reach their end of life on **September 27, 2026**" (deprecation announced 2026-04-10) — i.e. they are past EOL today; do not build on them.

Still dashboard-only per the registrar API docs: **renewals, transfer-out, contact updates, transfer-lock toggling** ("Renewals are not yet available through the API", "Contact updates are not yet available through the API"), premium-domain acknowledgements, and TLDs not in the API's extension list ("Some extensions supported in the dashboard are not yet available for programmatic registration"). The new `PATCH` only changes `auto_renew`, so `locked`/`privacy` are read-only via the new API (they were writable in the dead legacy one). Permission: a token with **Registrar Write**.

### 4. SSL/TLS — verdict: FULL

| Area | Endpoints |
|---|---|
| Certificate packs (Universal + Advanced) | `GET /zones/{id}/ssl/certificate_packs?status=all`, `GET …/{pack_id}`, `POST …/order` body `{certificate_authority: google|lets_encrypt|ssl_com, hosts[] (≤50, must include apex), type: advanced, validation_method: txt|http|email, validity_days, cloudflare_branding}`, `PATCH …/{pack_id}` (restart validation), `DELETE …/{pack_id}`, `GET …/quota` |
| Universal SSL | `GET/PATCH /zones/{id}/ssl/universal/settings` body `{enabled}` |
| Verification | `GET /zones/{id}/ssl/verification`, `PATCH …/verification/{pack_id}` (validation method) |
| Origin CA | `GET /certificates?zone_id=`, `POST /certificates` body `{csr, hostnames[] (≤200 SANs, wildcards ok), request_type: origin-rsa|origin-ecc|keyless-certificate, requested_validity: 7|30|90|365|730|1095|5475}`, `GET/DELETE /certificates/{id}` (DELETE = revoke). **Works with an ordinary API token** — this token listed them (200, `total_count: 0`); docs: "use an API token with Permissions that include Zone - SSL and Certificates - Edit" (https://developers.cloudflare.com/ssl/origin-configuration/origin-ca/). The TUI must generate the key + CSR locally (`rcgen`), Cloudflare never returns a private key |
| Custom certs / Origin pull / Total TLS | `GET/POST /zones/{id}/custom_certificates`, `/origin_tls_client_auth` (+ `/settings`), `GET/POST /zones/{id}/acm/total_tls`, `/acm/custom_trust_store` |
| Custom hostnames (SSL for SaaS) | `GET/POST /zones/{id}/custom_hostnames`, `GET/PATCH/DELETE …/{id}`, `/fallback_origin` GET/PUT/DELETE, `/quota`; create body `{hostname, ssl:{method, type, certificate_authority, bundle_method, wildcard, custom_certificate, custom_key, settings{min_tls_version,…}}, custom_origin_server, custom_origin_sni, custom_metadata}` — product requires the SaaS add-on |
| TLS versions / edge settings | zone settings `min_tls_version`, `tls_1_3`, `ciphers`, `ssl`, `always_use_https`, `automatic_https_rewrites`, `opportunistic_encryption`, `tls_client_auth`, `0rtt` |
| Edge certificate analysis | `POST /zones/{id}/ssl/analyze` (upload a cert to see what CF would do with it); `GET /zones/{id}/ssl/recommendation` is deprecated |

Dashboard-only: nothing material; the dashboard's "Edge Certificates" page is the pack list plus these settings. (Advanced Certificate Manager must be purchased first, via `/zones/{id}/subscription` or the dashboard.)

### 5. Cache — verdict: FULL

- Purge: `POST /zones/{id}/purge_cache` body is **one of** `{purge_everything:true}`, `{files:[url | {url, headers{}}]}`, `{tags:[…]}`, `{hosts:[…]}`, `{prefixes:["host/path"]}`. Spec description adds: purging also clears Cache Reserve, and there is a softer `POST /zones/{id}/invalidate_cache` ("keep content cached and have Cloudflare revalidate it with your origin instead"). Per https://developers.cloudflare.com/cache/how-to/purge-cache/ **all five purge modes are available on every plan now** (tags/hosts/prefixes used to be Enterprise-only); limits: hostname/tag/prefix/everything — 100 operations per request, **Free 5 requests/minute**, Pro 5/s, Business 10/s, Enterprise 50/s; single-file — 100 URLs per request (500 Enterprise), 800 URLs/s Free. Permission: **Cache Purge**.
- Tiered cache: `GET/PATCH /zones/{id}/argo/tiered_caching` `{value: on|off}`; Smart topology: `GET/PATCH/POST/DELETE /zones/{id}/cache/tiered_cache_smart_topology_enable`; Regional tiered cache: `GET/PATCH /zones/{id}/cache/regional_tiered_cache`.
- Cache Reserve: `GET/PATCH /zones/{id}/cache/cache_reserve` `{value}`, `GET/POST /zones/{id}/cache/cache_reserve_clear` (async clear + status).
- Variants: `GET/PATCH/DELETE /zones/{id}/cache/variants` (image format variants).
- Cache rules: Rulesets phase **`http_request_cache_settings`** (§6), action `set_cache_settings` with `action_parameters.cache`, `edge_ttl`, `browser_ttl`, `cache_key`, `serve_stale`, `respect_strong_etags`, `origin_error_page_passthru`, `cache_reserve`.
- Zone-level knobs: `cache_level`, `browser_cache_ttl`, `development_mode`, `always_online`, `sort_query_string_for_cache` settings; Argo smart routing `GET/PATCH /zones/{id}/argo/smart_routing`.

Dashboard-only: the "Cache Analytics" page = GraphQL `httpRequestsAdaptiveGroups` filtered on `cacheStatus`; Cache Reserve/Smart Shield purchase (subscription).

### 6. Rulesets engine + Page Rules — verdict: FULL

Zone (`/zones/{zone_id}/rulesets…`) and account (`/accounts/{account_id}/rulesets…`) share the same shape:

- `GET /rulesets` (list; only ids/phase/name/version, no rules), `POST /rulesets`, `GET/PUT/DELETE /rulesets/{ruleset_id}`
- Entry points: `GET/PUT /rulesets/phases/{phase}/entrypoint` (+ `/versions`, `/versions/{v}`)
- Per-rule: `POST /rulesets/{ruleset_id}/rules`, `PATCH/DELETE /rulesets/{ruleset_id}/rules/{rule_id}`; `GET /rulesets/{id}/versions/{v}/by_tag/{tag}` (for managed rules)
- Phase enum (spec): `ddos_l4, ddos_l7, http_config_settings, http_custom_errors, http_log_custom_fields, http_ratelimit, http_request_cache_settings, http_request_dynamic_redirect, http_request_firewall_custom, http_request_firewall_managed, http_request_late_transform, http_request_origin, http_request_redirect, http_request_sanitize, http_request_sbfm, http_request_transform, http_response_cache_settings, http_response_compression, http_response_firewall_managed, http_response_headers_transform, magic_transit, magic_transit_ids_managed, magic_transit_managed, magic_transit_ratelimit`.
  - Redirect rules = `http_request_dynamic_redirect` (action `redirect`); Bulk Redirects = account `http_request_redirect` + `/accounts/{id}/rules/lists` (type `redirect`); Transform rules = `http_request_transform` (URL rewrite), `http_request_late_transform` (request headers), `http_response_headers_transform`; Cache rules = `http_request_cache_settings`; Origin rules = `http_request_origin`; Configuration rules = `http_config_settings`; Compression = `http_response_compression`; WAF custom = `http_request_firewall_custom`; WAF managed = `http_request_firewall_managed` (action `execute` with `action_parameters.id` of the managed ruleset + `overrides`); Rate limiting = `http_ratelimit` (`ratelimit{characteristics, period, requests_per_period, mitigation_timeout}`); Super Bot Fight Mode = `http_request_sbfm`; custom error pages = `http_custom_errors`.
- The rule language is the Cloudflare expression syntax (`http.request.uri.path contains "/api"`); a TUI needs a text editor for `expression` and a form per `action`. Entitlement limits (e.g. 5 custom rules on Free) are reported as `errors` on PUT — surface them.
- Page Rules (legacy): `GET/POST /zones/{id}/pagerules`, `GET/PUT/PATCH/DELETE …/{pagerule_id}` (body `targets[]`, `actions[]`, `priority`, `status`), `GET …/pagerules/settings` is deprecated. Docs title now reads "Page Rules (deprecated)"; migration "is planned for late 2025 or beyond, with no action required on your part", with a per-setting mapping table (https://developers.cloudflare.com/rules/reference/page-rules-migration/). Still creatable today; show them read-mostly with a "migrate to Rules" hint.
- Deprecated and replaced (deprecation page, 2025-06-15): legacy Firewall Rules `/firewall/rules` + `/filters` → WAF custom rules; legacy rate limits `/rate_limits` → `http_ratelimit`; WAF packages `/firewall/waf/packages` → managed rulesets. The spec still carries `/zones/{id}/firewall/rules` as `deprecated: true`.

Dashboard-only: the rules "templates" gallery and the expression builder UI (pure front-end); Ruleset "versions" rollback is in the API (`/versions`).

### 7. Security — verdict: FULL

| Feature | Endpoint |
|---|---|
| WAF managed rules / overrides | rulesets phase `http_request_firewall_managed`; list managed rulesets with `GET /accounts/{id}/rulesets` (kind `managed`); per-rule overrides via `action_parameters.overrides.rules[]` |
| Custom rules | phase `http_request_firewall_custom` (actions `block|challenge|js_challenge|managed_challenge|log|skip`) |
| Rate limiting | phase `http_ratelimit` |
| IP Access Rules | `GET/POST /zones/{id}/firewall/access_rules/rules`, `GET/PATCH/DELETE …/{rule_id}`; account-wide `/accounts/{id}/firewall/access_rules/rules`; body `{mode: block|challenge|whitelist|js_challenge|managed_challenge, configuration:{target: ip|ip6|ip_range|asn|country, value}, notes}` |
| Zone Lockdown, UA blocking | `/zones/{id}/firewall/lockdowns`, `/zones/{id}/firewall/ua_rules` (CRUD) |
| Bot management | `GET/PUT /zones/{id}/bot_management` — base keys `ai_bots_protection, ai_search, ai_training, ai_user, crawler_protection, enable_js, is_robots_txt_managed, cf_robots_variant, jsd_api_results_enabled…`; plan-dependent extras `fight_mode` (Free: Bot Fight Mode on/off), `sbfm_definitely_automated|sbfm_likely_automated|sbfm_verified_bots` (Pro+), `auto_update_model|suppress_session_score` (Enterprise); `stale_zone_configuration` is read-only |
| Security level / Under Attack | `PATCH /zones/{id}/settings/security_level {value: under_attack}` (and back to `medium`); `challenge_ttl`, `browser_check` settings |
| Leaked credentials, content scanning | `GET/POST /zones/{id}/leaked-credential-checks` (+ `/detections` CRUD), `/zones/{id}/content-upload-scan/settings` |
| Turnstile | `GET/POST /accounts/{id}/challenges/widgets` (live: 2 widgets, keys `bot_fight_mode, clearance_level, created_on, domains[], ephemeral_id, mode, modified_on, name, offlabel, region, sitekey`), `GET/PUT/DELETE …/{sitekey}`, `POST …/{sitekey}/rotate_secret`; create body `{name, domains[], mode: non-interactive|invisible|managed, clearance_level: no_clearance|jschallenge|managed|interactive, bot_fight_mode, region, offlabel, ephemeral_id}`; the secret is only in create/rotate responses |
| Security events / analytics | GraphQL `firewallEventsAdaptive`, `firewallEventsAdaptiveGroups`, `firewallEventsAdaptiveByTimeGroups` (confirmed by schema introspection) |

Dashboard-only: "Security Insights"/"Security Center" at zone level returned `7000 No route` for the path tried (the product is account-scoped under `/accounts/{id}/security-center/insights` and in the spec); Turnstile widget analytics (dashboard charts only); DDoS override rules exist in the API (`ddos_l7` phase) but are Enterprise.

### 8. Zero Trust — verdict: FULL for Access + Tunnels; PARTIAL for WARP devices (list/revoke only)

Access (all live with this token):
- Apps: `GET /accounts/{id}/access/apps` (query `name, domain, aud, search, page, per_page ≤1000 default 1000`), `POST`, `GET/PUT/DELETE …/{app_id}`. Live keys: `id, uid, type (self_hosted|saas|ssh|vnc|…), name, aud, domain, self_hosted_domains[], destinations[], allowed_idps[], app_launcher_visible, auto_redirect_to_identity, session_duration ("24h"), enable_binding_cookie, http_only_cookie_attribute, options_preflight_bypass, eager_redirect_cookie_setting, tags[], policies[]`.
- Policies: per-app `GET/POST /access/apps/{app_id}/policies`, `GET/PUT/DELETE …/{policy_id}` (keys `decision allow|deny|non_identity|bypass, include[], exclude[], require[], precedence, name`); reusable `GET/POST /access/policies` (+ `approval_required, purpose_justification_*, session_duration, isolation_required, connection_rules`). Groups: `/access/groups` (needs Access: Organizations, Identity Providers, and Groups Read — this token lacked it). IdPs: `/access/identity_providers`.
- Service tokens: `GET/POST /access/service_tokens` (`per_page ≤1000`), `GET/PUT/DELETE …/{id}`, `POST …/{id}/refresh` (extend expiry), `POST …/{id}/rotate` (new secret; `previous_client_secret_expires_at` grace). **`client_secret` is returned only by the 201 of POST and by rotate** — show once and offer to copy.

Tunnels (`cfd_tunnel`; the older `/accounts/{id}/tunnels` is deprecated since 2024-02-04 but still answers):
- `GET /accounts/{id}/cfd_tunnel` (query `name, is_deleted, uuid, status inactive|degraded|healthy|down, was_active_at, include_prefix, per_page ≤1000`), `POST` body `{name (required), config_src: local|cloudflare, tunnel_secret (≥32 bytes base64, only for local)}` — response includes `id`, `token` and a `credentials_file`. Live GET keys: `account_tag, config_src, connections[], conns_active_at, conns_inactive_at, created_at, deleted_at, id, metadata, name, remote_config, status, tun_type`.
- `GET …/{tunnel_id}/token` → the base64 JSON blob `{a: account_tag, t: tunnel_id, s: secret}` that `cloudflared tunnel run --token …` consumes ("Treat the token as a secret").
- `GET/PUT …/{tunnel_id}/configurations` body `{config:{ingress:[{hostname, path, service, originRequest{…}}, {service:"http_status:404"}], originRequest{connectTimeout, noTLSVerify, http2Origin, access{audTag[], teamName, required}, …}, warp-routing{enabled}}}` — last ingress rule must be the catch-all. Plus a DNS CNAME `<tunnel_id>.cfargotunnel.com` proxied (via §2), which the dashboard's "Public hostname" tab does for you — the TUI must do both calls. Doc walk-through: https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel-api/ (permissions: Cloudflare One Connectors Write / Cloudflare Tunnel Write).
- `GET …/{tunnel_id}/connections`, `DELETE …/connections?client_id=` ("If no connector id is provided all connectors will be removed. We recommend running this command after rotating tokens"), `GET …/connectors/{connector_id}`, `POST …/{tunnel_id}/management` (management token for `cloudflared tail`), `PATCH …/{tunnel_id}` `{name, tunnel_secret}` (rotate), `DELETE …/{tunnel_id}` — "The tunnel must have no active connections" (clean up connections first, then delete).
- Private network routes: `/accounts/{id}/teamnet/routes`, virtual networks `/teamnet/virtual_networks`.

WARP devices: `GET /accounts/{id}/devices` and `…/devices/{id}` are **deprecated with EOL 2025-11-11**; use `GET /accounts/{id}/devices/physical-devices` (cursor, `sort_by`, `search`, `has_registration_type warp|browser_extension`) and `GET/DELETE /accounts/{id}/devices/registrations` (revoke). Device posture, settings policies (`/devices/policies`, `/devices/settings`) and Gateway rules (`/gateway/rules`) are all in the API but need their own scopes (403 here). Enrolment itself is client-side.

### 9. Traffic — verdict: FULL (brief)

- Load balancers: zone-level `GET/POST /zones/{id}/load_balancers`, `GET/PUT/PATCH/DELETE …/{lb_id}` (body `name, default_pools[], fallback_pool, steering_policy, proxied, ttl, session_affinity*, region_pools, country_pools, pop_pools, rules[], adaptive_routing, random_steering, location_strategy`); account-level LBs `/accounts/{id}/load_balancers`; pools `GET/POST/PATCH /accounts/{id}/load_balancers/pools`, `GET/PUT/PATCH/DELETE …/{pool_id}` (+ `/health`, `/references`), monitors `/accounts/{id}/load_balancers/monitors` (+ `/preview`), previews `/accounts/{id}/load_balancers/preview/{id}`, regions `/load_balancers/regions`. Requires the LB subscription.
- Standalone health checks: `GET/POST /zones/{id}/healthchecks`, `GET/PUT/PATCH/DELETE …/{id}`, `POST /zones/{id}/healthchecks/preview`.
- Spectrum: `GET/POST /zones/{id}/spectrum/apps`, `GET/PUT/DELETE …/{app_id}`, analytics under `/zones/{id}/spectrum/analytics/…` (paid add-on).
- Argo: `GET/PATCH /zones/{id}/argo/smart_routing {value}`, latency analytics `GET /zones/{id}/analytics/latency` and `/latency/colos` (these two REST analytics endpoints are *not* deprecated).
- Waiting Room: `/zones/{id}/waiting_rooms` CRUD (+ events, rules, settings).

### 10. Analytics — verdict: PARTIAL (REST is deprecated; GraphQL is complete but permission- and plan-gated)

- REST `GET /zones/{id}/analytics/dashboard` and `/analytics/colos`: **deprecated since 2021-03-01**, no sunset date; this token got 403 anyway. `GET /zones/{id}/dns_analytics/report` (+ `/bytime`): **deprecated 2025-12-09, sunset 2026-12-01** → GraphQL `dnsAnalyticsAdaptive(Groups)`.
- GraphQL `POST https://api.cloudflare.com/client/v4/graphql` (same bearer token; needs **Analytics Read** — the error returned names the missing permission `com.cloudflare.api.account.zone.analytics.read`). Introspection (works without data permissions) lists on `zone`: `httpRequests1mGroups, httpRequests1hGroups, httpRequests1dGroups, httpRequests1mByColoGroups, httpRequests1dByColoGroups, httpRequestsAdaptive, httpRequestsAdaptiveGroups, httpRequestsOverviewAdaptiveGroups, firewallEventsAdaptive, firewallEventsAdaptiveGroups, firewallEventsAdaptiveByTimeGroups, dnsAnalyticsAdaptive(Groups), healthCheckEventsAdaptive(Groups), loadBalancingRequestsAdaptive(Groups), cacheReserve*AdaptiveGroups, workersZoneInvocationsAdaptiveGroups, workersZoneSubrequestsAdaptiveGroups`; on `account`: `workersInvocationsAdaptive, workersInvocationsScheduled, workersOverview*`, `d1AnalyticsAdaptiveGroups, d1QueriesAdaptiveGroups, d1StorageAdaptiveGroups`, `r2OperationsAdaptiveGroups, r2StorageAdaptiveGroups, r2BandwidthUsageAdaptiveGroups`, `queueBacklogAdaptiveGroups, queueConsumerMetricsAdaptiveGroups, queueMessageOperationsAdaptiveGroups`, `kvOperationsAdaptiveGroups, kvStorageAdaptiveGroups`, `durableObjects*`, `pagesFunctionsInvocationsAdaptiveGroups`, `rumPageloadEventsAdaptiveGroups, rumPerformanceEventsAdaptiveGroups, rumWebVitalsEventsAdaptive(Groups)`, `accessLoginRequestsAdaptiveGroups`, `gateway*`, `workersBuildsBuildMinutesAdaptiveGroups`, and a `settings` node for per-dataset limits.
- The dashboard "Traffic" tab is `httpRequestsAdaptiveGroups` (dimensions `datetimeHour|datetimeFifteenMinutes|clientCountryName|cacheStatus|edgeResponseStatus|clientRequestHTTPHost…`, `sum{edgeResponseBytes, visits}`, `count`); older per-minute rollups are `httpRequests1mGroups` (`sum{requests, bytes, cachedRequests, threats, pageViews}`, `uniq{uniques}`). Retention: since **2026-10-02** "adaptive analytics datasets, such as HTTP requests, security events, and DNS analytics, retain at least 31 days of data for Free and Pro domains, and you can query up to 30 days in a single request"; "aggregated datasets, such as `httpRequests1hGroups`, keep their existing per-plan limits" (query `viewer.zones.settings` / `accounts.settings` for exact per-dataset windows) — https://developers.cloudflare.com/changelog/post/2026-10-02-30-days-analytics-on-every-plan/ . Default quota: 300 GraphQL queries per 5-minute window (https://developers.cloudflare.com/analytics/graphql-api/limits/).
- Web Analytics (RUM) sites: `GET /accounts/{id}/rum/site_info/list` (live, 5 sites, `per_page` honoured, keys `site_tag, site_token, snippet, auto_install, created, ruleset{id, enabled, lite, zone_name, zone_tag}` — note the literal `"id_please_delete"` key, serialize loosely), `POST /rum/site_info`, `GET/PUT/DELETE …/site_info/{site_id}`, rules under `/rum/v2/{ruleset_id}/rules`; the data itself is GraphQL `rumPageloadEventsAdaptiveGroups` etc.

Dashboard-only: the Zone Overview sparkline and "Analytics & Logs" dashboards are GraphQL compositions — reproducible but each panel is a query; "Logs" (Logpush) is API; Instant Logs is a WebSocket (`/zones/{id}/logpush/edge/jobs`, Enterprise).

### 11. Account & admin — verdict: PARTIAL (members/tokens/audit/notifications FULL; billing mostly read-only, payment methods need a browser)

- Members: `GET /accounts/{id}/members` (live: `per_page` 5–50 default 20, `status accepted|pending|rejected`, `order user.email|…`; keys `id, email, status, api_access_enabled, roles[{id,name,…}], policies[{id, access, permission_groups[], resource_groups[]}], user{email, first_name, last_name, id, two_factor_authentication_enabled}`), `POST` body **either** `{email, roles:[role_id…], status}` **or** `{email, policies:[{access: allow, permission_groups:[{id}], resource_groups:[{scope…}]}]}`, `GET/PUT/DELETE …/{member_id}`.
- Roles: `GET /accounts/{id}/roles` (live: 148 roles, 8 pages, keys `id, name, description, permissions{access, organization, zone…}`) — **deprecated 2026-07-21** "in favor of the Permission Groups API": `GET /accounts/{id}/iam/permission_groups` (+ `/{id}`), resource groups `GET/POST/PUT/DELETE /accounts/{id}/iam/resource_groups`, user groups `/accounts/{id}/iam/user_groups` (+ `/members`). Memberships from the user side: `GET /memberships`, `PUT/DELETE /memberships/{id}`; invites `GET /user/invites`, `PATCH /user/invites/{id}` (accept/reject).
- API tokens: user-owned `GET /user/tokens` (`per_page` 5–50, `include_expired`), `POST /user/tokens` body `{name (≤120), policies:[{effect: allow|deny, permission_groups:[{id, meta}], resources:{"com.cloudflare.api.account.<id>":"*", "com.cloudflare.api.account.zone.<zone_id>":"*"}}], condition:{request_ip:{in[], not_in[]}}, expires_on, not_before}`; response includes **`value` once** (plus `id, status, policies, issued_on, last_used_on, creator_email_at_creation, provisioner_type`); `GET/PUT/DELETE /user/tokens/{id}`; **roll** `PUT /user/tokens/{id}/value` (returns the new secret); `GET /user/tokens/verify`; `GET /user/tokens/permission_groups?name=&scope=` (list of `{id, name, scopes[]}` to build the picker). Account-owned tokens mirror all of this under `/accounts/{id}/tokens…` (`/verify`, `/permission_groups`, `/{id}/value`). Needs **API Tokens Read/Edit** (this token got 9109 on every one of them).
- Audit logs: v1 `GET /accounts/{id}/audit_logs` (live; `since`, `before`, `action.type`, `actor.email`, `actor.ip`, `zone.name`, `hide_user_logs`, `export`, `per_page` ≤1000 default 100, page-numbered; entries `{id, action{type, info, result}, actor{type, email, id, ip}, resource{type, id}, interface, metadata{}, oldValue/newValue(+Json), owner, when}`); **v2** `GET /accounts/{id}/logs/audit` (live; **cursor** pagination `result_info:{count, cursor}`, `limit` ≤1000, ~45 filterable array params with `.not` negations: `actor_email, actor_token_id, action_type, action_result, resource_product, resource_type, zone_id, raw_uri, raw_method, raw_status_code…`; entries `{id, account{}, action{}, actor{}, raw{}, resource{}}`), `GET /logs/audit/product_categories`, `GET /logs/audit/{id}/history`. User-level `GET /user/audit_logs` also exists.
- Billing (all 403 for this token; needs **Billing Read/Edit**): `GET/POST/PUT/PATCH/DELETE /accounts/{id}/billing/profile` (PATCH = billing email), `GET /accounts/{id}/billing/history` (items `{id, type, action, description, amount, amount_to_pay, currency, occurred_at, status, invoice_id, receipt_id, external_invoice_id, hosted_invoice_url, source}`), `GET /accounts/{id}/receipts/{receipt_id}/pdf` (PDF download), `GET /accounts/{id}/billing/unpaid-invoice` (returned `7000 No route` today — spec is ahead of production), `POST /accounts/{id}/pay-invoice` (returns a Stripe client secret when SCA is required), `PATCH /accounts/{id}/invoices` (toggle PDF invoices), `GET /accounts/{id}/billing/credits`, `/billing/bad-debt`, `GET /accounts/{id}/billing/usage?metrics=&since=&until=&time_delta=hour|day|…&limit≤10000` ("time-series data for all billable product metrics including Stream, Images, Rate Limiting, Load Balancing, Argo, Workers, Workers KV, Image Resizing, Spectrum"); subscriptions `GET/POST /accounts/{id}/subscriptions`, `GET/PUT/DELETE …/{subscription_id}` (items `{id, app, rate_plan{id, public_name, currency, scope, sets[]}, component_values[], price, currency, frequency, state, current_period_start/end, zone}`), `GET/POST/PUT/DELETE /zones/{id}/subscription`, `GET /user/subscriptions`. `GET /user/billing/profile` and `/user/billing/history` are deprecated (2023-06-06) with "No API replacement". `/user/invoices` does not exist (7000).
  - **Dashboard-only**: adding/changing a card — `POST /accounts/{id}/billing/profile/payment-method` only "Creates a Stripe payment intent … Returns a client secret for frontend payment method collection", i.e. the TUI would have to open a browser; tax IDs/VAT; cancelling the account; invoices are downloadable via API only as receipts (`/receipts/{id}/pdf`) once `billing/history` gives the `receipt_id`.
- Notifications (live): `GET/POST /accounts/{id}/alerting/v3/policies`, `GET/PUT/DELETE …/{policy_id}` (live policy: `{alert_type: billing_budget_alert, enabled, filters{total_spend_dollars:[…]}, mechanisms{email:[{id}], webhooks:[{id}], pagerduty:[{id}]}, name, description, alert_interval}`), `GET /alerting/v3/available_alerts` (live: grouped by product — Abuse, Access, Billing, DoS Protection, Health Checks, Load Balancing, Logpush, Pages, SSL/TLS, Stream, Traffic Monitoring, Workers…, each with `type, display_name, description, filter_options[]`), webhooks `GET/POST /alerting/v3/destinations/webhooks`, `GET/PUT/DELETE …/{webhook_id}` (body `{name, url, secret}` → `cf-webhook-auth` header), `GET /alerting/v3/destinations/eligible` (live: `{email, pagerduty, webhooks}` with per-plan eligibility), PagerDuty connect flow (`/destinations/pagerduty/connect`), `GET /alerting/v3/history` (`per_page` 5–1000, `since/before`).
- Account itself: `GET /accounts/{id}` (live keys `created_on, id, legacy_flags, name, settings, type`), `PUT /accounts/{id}` (name, settings such as `enforce_twofactor`), `GET /accounts` (list, needs user-level token or membership). `GET /user` / `PATCH /user` need **User Details**.

Dashboard-only across the board: anything that ends in a Stripe card form, account deletion, "Super Administrator" transfer, SSO/SCIM provisioning setup (SCIM has an API but needs Enterprise), and the dashboard's "Notifications → test webhook" (the API has no test call; send a real alert or POST to the webhook yourself).

---

## PART 2 — Rust ecosystem check (2026-10-05; versions from crates.io API, `cargo info`, GitHub)

### a. Cloudflare API crates — verdict: write a thin reqwest client; do not build on `cloudflare` 0.14

**`cloudflare` (cloudflare/cloudflare-rs)** — https://crates.io/crates/cloudflare , https://github.com/cloudflare/cloudflare-rs
- Latest published: **0.14.0 on 2025-03-13** (0.13.0 2025-02-04, 0.12.0 2025-01-28, 0.11.0 2023-10-12). 699k downloads total.
- Last commit on master **2025-08-08** ("Release v0.14.1") — bumps Cargo.toml to 0.14.1 but **0.14.1 was never published**. No merged community PR since 2025-05-30. 29 open PRs (incl. "Upgrade to reqwest 0.13" open since 2026-06-04, DNS record-type fixes, cache purge, Workers schedules), ~40 open issues. README still says "⚠️ This library is a Work in Progress! ⚠️".
- Pins `reqwest = "0.12.12"` (`default-features=false, features=["json","multipart"]`; crate features `default-tls`, `rustls-tls`, `blocking`). reqwest is at 0.13.5 (2026-09-08), so you end up with two HTTP/TLS stacks.
- Coverage (`cloudflare::endpoints`, https://docs.rs/cloudflare/latest/cloudflare/endpoints/index.html): `account` (list, user details, token verify), `zones` (list/get/create only), `dns` (list/create/PUT/delete, no PATCH/export/import, incomplete record enum), `workers` (routes, secrets, **tail** create/list/delete/heartbeat, bindings, delete script — no upload/versions/deployments/subdomain/crons), `workerskv` (full), `r2` (list/create/delete bucket — no objects), `ai` (ExecuteModel), `argo_tunnel`/`cfd_tunnel`, `load_balancing`. **Nothing for D1, Queues, Workers versions/deployments, GraphQL analytics, Rulesets, Zero Trust Access/Gateway, Pages, Turnstile, cache purge, zone settings, tokens, billing.** Workers tail returns the websocket URL only; no WS client.
- Extensible via `cloudflare::framework::endpoint::EndpointSpec` (`method()`, `path()`, `query()`, `body()` → `RequestBody::{Json,Raw,MultiPart}`, `type JsonResponse`, `type ResponseType`) and `Client::request<E: EndpointSpec>()`; the `ApiSuccess{result, result_info, messages, errors}` / `ApiFailure` envelope is reusable. Known bug: `errors: null` from some endpoints fails to deserialise (open PR 2026-02-09).
- Other crates: `cloudflare-rs` 0.7.0 (2021-08-17, dead fork), `cloudflare-api`/`cloudflare_api` 0.1.0 (2026-03-25, placeholders), `cloudflare-r2-rs` 0.7.0 (2026-01-11, R2 object helper), `cf-turnstile` 0.2.0 (verification only), `worker` 0.8.7 (workers-rs, runtime SDK not management API). `cf-api`, `cloudflare-sdk`, `cloudflare-client` do not exist. New and unproven: **cf-contrib/cloudflare-rs** (https://github.com/cf-contrib/cloudflare-rs, created 2026-10-02) — build-script codegen over the full OpenAPI with one feature per product, git-only ("won't be published to crates.io"), "compiling every product takes some 10 minutes and over 10 GB of memory"; usable as a source of types by `rev`, not as a foundation.
- No Stainless/Fern official Rust SDK exists (Cloudflare ships Go/TS/Python only).

Verdict: with ~90% of the TUI's surface (D1, R2 objects, Queues, versions/deployments, Rulesets, Zero Trust, GraphQL, tokens, billing) missing, the crate buys a 200-line envelope and a reqwest-0.12 pin. Copy its `EndpointSpec` shape, the envelope struct (with `errors: Option<Vec<_>>`), and page/cursor `result_info` handling into your own reqwest 0.13 client; lift large request/response structs straight from `cloudflare/api-schemas` where needed.

### b. TUI stack

| Crate | Version | Published | Depends on / notes |
|---|---|---|---|
| **ratatui** | **0.30.2** | 2026-06-19 | MSRV 1.88, edition 2024; 0.30.0 was 2025-12-26; no 0.31 yet (BREAKING-CHANGES.md on main already has a "v0.31.0" heading: `Backend` gains default `save_cursor_position`/`restore_cursor_position`). Default features `all-widgets, crossterm, layout-cache, macros, underline-color`. Facade over `ratatui-core 0.1.2`, `ratatui-widgets 0.3.2`, `ratatui-crossterm 0.1.2` (features `crossterm_0_28`/`crossterm_0_29`, default 0.29), `ratatui-termion/-termwiz/-termina`, `ratatui-macros 0.7.2`. https://crates.io/crates/ratatui |
| **crossterm** | **0.29.0** | 2025-04-05 | 0.28→0.29 is mechanical (adds OSC52 clipboard, `Event::is_key_press()` helpers, `query_keyboard_enhancement_flags`; only breaking: `KeyModifiers` Display). Unreleased master already lists MSRV 1.85, edition 2024, `IsTty` trait removed (use `std::io::IsTerminal`). https://github.com/crossterm-rs/crossterm/blob/master/CHANGELOG.md |
| tui-textarea | 0.7.0 | 2024-10-22 | **ratatui ^0.29 / crossterm ^0.28 — does not work with 0.30** (PR to bump crossterm open since 2025-12-02). Use **`ratatui-textarea` 0.9.2** (2026-06-12, ratatui org fork, depends on ratatui-core ^0.1.1 + ratatui-widgets ^0.3.1, https://github.com/ratatui/ratatui-textarea) or `tui-textarea-2` 0.13.2 (2026-08-23) or `edtui` 0.11.7 (vim-style). |
| tui-tree-widget | 0.24.1 | 2026-08-09 | ratatui-core ^0.1 + ratatui-widgets ^0.3 — OK |
| tui-input | 0.15.5 | 2026-09-26 | ratatui ^0.30.2, crossterm ^0.29 — OK |
| tui-scrollview | 0.6.8 | 2026-09-24 | ratatui-core/widgets — OK (ratatui/tui-widgets repo) |
| tui-popup | 0.7.7 | 2026-09-24 | OK |
| **ratatui-image** | 11.1.0 (2026-09-17); 12.0.0-rc.2 (2026-10-05) | | ratatui ^0.30.1, image ^0.25.6, icy_sixel. Protocols: **Sixel, Kitty, iTerm2, halfblocks fallback** (+ optional chafa). `Picker::from_query_stdio()` auto-detects. Works: Kitty, Ghostty (Kitty), WezTerm (iTerm2), iTerm2, Rio, foot/xterm/mlterm (Sixel). **Not working: Alacritty, Konsole, Contour, Warp.** 12.0 rc changes Kitty placement and adds `Picker::tmux_detected()`. https://github.com/ratatui/ratatui-image |
| tui-widget-list 0.15.3 (2026-07-18), throbber-widgets-tui 0.11.1 (2026-06-19), tui-logger 0.18.3 (2026-07-04), tachyonfx 0.25.2 (2026-09-06) | | | all on ratatui-core 0.1 / ratatui 0.30 — OK |

Templates: `cargo generate ratatui/templates` (https://github.com/ratatui/templates, last push 2026-09-06) pin `ratatui = "0.30.2"`, `crossterm = "0.29.0"`, `color-eyre = "0.6.5"`, async variants `tokio = "1.52.3"` + `tokio-util 0.7.18`; `component` template enables ratatui `["serde","macros"]`, crossterm `["serde","event-stream"]`.

**Breaking changes 0.29 → 0.30.x that bite a 0.29 codebase** (https://github.com/ratatui/ratatui/blob/main/BREAKING-CHANGES.md , https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md , https://ratatui.rs/highlights/v030/):
1. MSRV 1.86 (0.30.0) → **1.88** (0.30.1+), edition 2024 (#1863, #2230).
2. **crossterm 0.29** by default; `From`/`Into` between crossterm and ratatui `Color`/`Modifier`/`Style` replaced by **`FromCrossterm`/`IntoCrossterm`** traits (#1464) — `Color::from_crossterm(c)` not `c.into()`.
3. **`Block` titles** (#1926): `widgets::block::Title` and `block::Position` removed; `Block::title()` takes `Into<Line>`; use `.title_bottom(..)` / `TitlePosition`.
4. `Alignment` → **`HorizontalAlignment`** (#1735; alias still exported), new `VerticalAlignment`.
5. `Style` no longer implements `Styled` (#1572); `Stylize` methods are inherent on `Style` and usable in `const`.
6. **`Backend` trait**: associated `type Error` replaces `io::Error`, new required `clear_region()` (#1778, #1823); `Terminal::draw` returns `Result<CompletedFrame, B::Error>` — generic `fn run<B: Backend>` signatures must change; `CrosstermBackend::Error = io::Error` so concrete code compiles.
7. `Flex::SpaceAround` semantics changed (old = `Flex::SpaceEvenly`) (#1952).
8. `Marker` is `#[non_exhaustive]` (#2236) — add `_ =>`.
9. `List::highlight_symbol` takes `Into<Line>` and is no longer `const` (#1595); `Bar::label`/`BarGroup::label` take `Into<Line>` (#1471).
10. `WidgetRef` blanket impl reversed (#1491/#1530): implement `Widget for &MyWidget` instead of `WidgetRef`; `Frame::render_widget_ref` moved to `FrameExt` behind `unstable-widget-ref`.
11. `layout-cache` is now a (default-on) feature; `StatefulWidget::State: ?Sized`; termwiz 0.23; unicode-width 0.2.1 (widths of some symbols change).

Unchanged (verified in 0.30.2 sources): `Frame::render_widget(widget, area)` by value, `Terminal::draw(|f| ..)` closure, `Widget::render(self, area, buf)` by value, `Layout::split -> Rc<[Rect]>` plus `areas::<N>()`, `Block::bordered()`, `Paragraph::wrap(Wrap{trim})`, `ListState`/`TableState` (with the 0.27–0.29 `select_next/previous/first/last`, `scroll_*_by`, `select_column/cell` additions), `ratatui::init()/restore()`; `Frame::size()` is still only deprecated (use `area()`). **New in 0.30: `ratatui::run(|terminal| ..)`** (#1707), `Rect::layout()/layout_vec()/centered*()`, `try_areas::<N>()`, `Fill` widget, block shadows, dashed borders, table column spans, Termina backend (0.30.2). `ratatui-macros` (`span!`, `line!`, `text!`, `row!`, `constraints!`, `horizontal!/vertical!`) is re-exported as `ratatui::macros` under the default `macros` feature.

### c. HTTP client, TLS backend, WebSocket client

Versions (crates.io, 2026-10-05): reqwest **0.13.5** (2026-09-08), tokio 1.53.2, hyper 1.11.1, hyper-rustls 0.27.10, rustls 0.23.45, ring 0.17.14 (2025-03-11; repo commits through 2026-07), aws-lc-rs 1.18.1, ureq 3.4.2, tokio-tungstenite 0.30.0 (2026-07-11), async-tungstenite 0.35.0, fastwebsockets 0.10.0.

**reqwest 0.13 renamed the TLS features** (https://raw.githubusercontent.com/seanmonstar/reqwest/v0.13.5/CHANGELOG.md): rustls is the default backend; **`rustls-tls` → `rustls`**; the crypto provider defaults to **aws-lc-rs**; `rustls-tls-webpki-roots`/`rustls-tls-native-roots` are **removed** — roots come from `rustls-platform-verifier` (macOS Security.framework, Windows CryptoAPI, Linux system bundle via rustls-native-certs/openssl-probe); to ship bundled roots call `ClientBuilder::tls_certs_only(...)`; `query`/`form` are now opt-in features. Feature table from `v0.13.5/Cargo.toml`: `default = ["default-tls","charset","http2","system-proxy"]`, `rustls = ["__rustls-aws-lc-rs", "dep:rustls-platform-verifier", "__rustls"]`, **`rustls-no-provider`** (rustls without a crypto provider), `json`, `stream`, `gzip|brotli|zstd|deflate`, `http2`, `blocking`. To use ring:

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls-no-provider", "json", "stream", "gzip", "http2"] }
rustls  = { version = "0.23", default-features = false, features = ["ring", "std", "tls12", "logging"] }
tokio   = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
```
```rust
fn main() { rustls::crypto::ring::default_provider().install_default().expect("crypto provider"); /* … */ }
```
rustls auto-selects the provider only when exactly one of `ring`/`aws_lc_rs` is enabled; if any transitive dep enables `rustls/aws-lc-rs` the first TLS use panics ("Could not automatically determine the process-level CryptoProvider…") — hence always `install_default()` and run `cargo tree -e features -i aws-lc-rs` in CI (https://raw.githubusercontent.com/rustls/rustls/v/0.23.45/rustls/src/crypto/mod.rs, https://github.com/algesten/ureq/issues/751). Add `system-proxy` back if corporate proxies matter.

**Measured sizes** (x86_64-linux-gnu, rustc 1.97.1, lazymongo's profile `opt-level="z", lto, codegen-units=1, strip, panic=abort`): hello 0.29 MB; ureq 3.4 (ring+webpki-roots, json, gzip) 1.62 MB; reqwest 0.13 + ring, HTTP/1 2.06 MB; + `http2` 2.27 MB; + tokio-tungstenite 0.30 2.42 MB; reqwest 0.13 + **aws-lc-rs** 3.04 MB. So ring saves ~0.8 MB with size flags; at default `opt-level=3` without LTO aws-lc-rs costs "~4 MB" (https://github.com/aws/aws-lc-rs/issues/745). The "3–6 MB" guess is right for a plain release profile; with the size profile budget ~2.5 MB for HTTP+WS+ratatui.

Alternatives: **ureq 3.4.2** (blocking; `default = ["rustls","gzip"]` where `rustls = ["rustls-no-provider","_ring","rustls-webpki-roots"]`, i.e. ring + webpki-roots by default, `platform-verifier` optional; https://raw.githubusercontent.com/algesten/ureq/3.4.2/Cargo.toml) — smallest, but you still need an async stack or blocking `tungstenite` for the tail; **hyper + hyper-rustls 0.27.10** direct (default features include `aws-lc-rs`, `ring` alternative) saves ~300 KB and loses redirects/decompression/timeouts/proxies/multipart; isahc 2.0.1 (libcurl, hurts static musl), attohttpc 0.31.0 (blocking, `tls-rustls`), minreq 3.0.0 (HTTP/1, no streaming). **Recommendation: reqwest 0.13 + ring + tokio rt-multi-thread**; one HTTP stack for REST, GraphQL, OAuth and R2.

**Workers tail WebSocket** (wrangler's `packages/wrangler/src/tail/createTail.ts`, https://raw.githubusercontent.com/cloudflare/workers-sdk/main/packages/wrangler/src/tail/createTail.ts): (1) `POST /accounts/{id}/workers/scripts/{name}/tails` with body `{"filters":[…]}` → `{id, url (wss://tail.developers.workers.dev/…), expires_at}` (permission "Workers Tail Read" or "Workers Scripts Write"; OAuth scope `workers_tail:read`); (2) connect with subprotocol **`trace-v1`** (`Sec-WebSocket-Protocol: trace-v1`, plus a `User-Agent`); (3) on open **send the text frame `{"debug":false}`** or the stream never starts (filters are the ones POSTed at creation); (4) `DELETE …/tails/{id}` when done. Filter shapes: `{"sampling_rate":0.25}`, `{"outcome":["ok","canceled","exception","exceededCpu","exceededMemory","unknown"]}`, `{"method":["GET"]}`, `{"header":{"key","query"}}`, `{"client_ip":[…]}`, `{"query":"text"}`, `{"scriptVersion":"…"}`. Limits: 10 concurrent viewers per Worker; sampling under high traffic (https://developers.cloudflare.com/workers/observability/logs/real-time-logs/).

Crate: **tokio-tungstenite 0.30.0 with `rustls-tls-webpki-roots`** — its `rustls`/`tokio-rustls` deps are `default-features = false`, so it reuses the installed ring provider and adds only ~150 KB (https://raw.githubusercontent.com/snapview/tokio-tungstenite/v0.30.0/Cargo.toml). Subprotocol via `tungstenite::client::ClientRequestBuilder`:
```rust
let req = ClientRequestBuilder::new(tail.url.parse::<http::Uri>()?)
    .with_sub_protocol("trace-v1")
    .with_header("User-Agent", concat!("cf-tui/", env!("CARGO_PKG_VERSION")));
let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await?;
ws.send(Message::Text(r#"{"debug":false}"#.into())).await?;
while let Some(msg) = ws.next().await { match msg? { Message::Text(t) => { /* TailEventMessage */ } Message::Close(_) => break, _ => {} } }
```
async-tungstenite 0.35.0 (features `tokio-rustls-webpki-roots|-native-certs|-platform-verifier|-manual-roots`) only if you want the platform verifier for WS too; fastwebsockets 0.10.0 is a frame codec (you do TLS + handshake yourself); blocking `tungstenite::connect` if you go the ureq route.

### d. R2 over the S3 API without the AWS SDK

| crate | version (date) | HTTP | notes |
|---|---|---|---|
| **rusty-s3** | 0.10.2 (2026-08-01) | none (sans-IO) | SigV4 **query-string presigning** of CreateBucket/DeleteBucket/HeadBucket, HeadObject, GetObject, PutObject, DeleteObject(s), **ListObjectsV2**, multipart (Create/UploadPart/Complete/Abort/ListParts); `S3Action::sign(expires) -> Url`, extra headers/query get signed; response XML parsing behind `full`/`xml`; crypto via `rustcrypto` (hmac+sha2) by default — no ring/aws-lc of its own; MSRV 1.85; Minio-tested. https://github.com/paolobarbolini/rusty-s3 |
| rust-s3 (`s3`) | 0.38.0 (2026-10-04) | **reqwest 0.12** async / attohttpc sync | most complete (presign, list, streaming get/put, multipart, copy, tags) but its rustls features reference `reqwest/rustls-tls` (0.12 names) → second hyper/TLS stack next to reqwest 0.13; choose `tokio-rustls-tls-ring` to avoid aws-lc |
| object_store | 0.14.2 (2026-09-15) | reqwest 0.13 | `aws` feature = aws-lc-rs; use `["aws-base","reqwest","ring"]`; `AmazonS3Builder` parses `https://ACCOUNT.r2.cloudflarestorage.com/bucket` (region auto, endpoint set); list/get(range)/put/put_multipart/delete/copy/signed_url; no checksum by default; heavy (quick-xml, chrono, tracing) |
| s3-simple | 0.9.1 (2026-09-21) | reqwest 0.13 | header-signed CRUD + list + copy + streaming; deliberately no presigning; one maintainer |
| aws-sdk-s3 + aws-config | 1.152.0 (2026-10-01) + 1.12.0 | hyper 1 via aws-smithy-http-client | `default-https-client` = aws-lc-rs; ring only via `aws-smithy-http-client` `rustls-ring` + `tls_provider(Rustls(CryptoMode::Ring))`; needs `BehaviorVersion` (`behavior-version-latest` or `v2023_11_09()`); 55 direct deps, minutes of clean build; measured 3.61 MB (ring, modern client) / 4.38 MB (aws-lc) / 5.50 MB (legacy `rustls` feature) — see below; published: +~4 MB from aws-lc alone, 5–15 MB typical SDK binaries |

R2 with aws-sdk-s3 (official page https://developers.cloudflare.com/r2/examples/aws/aws-sdk-rust/ , modified 2026-04-21): `aws_config::from_env().endpoint_url("https://{account}.r2.cloudflarestorage.com").credentials_provider(Credentials::new(key, secret, None, None, "R2")).region("auto")`. **The checksum quirk is real but missing from the Rust page**: SDK ≥1.69.0 (2025-01-15) sends CRC32 request checksums and validates response checksums by default (https://github.com/awslabs/aws-sdk-rust/issues/1240); R2 does not implement `x-amz-sdk-checksum-algorithm`/`x-amz-checksum-algorithm` on PutObject/CreateMultipartUpload/CopyObject (https://developers.cloudflare.com/r2/api/s3/api/, updated 2026-07-31) and Cloudflare's Java/Ruby pages tell you to set `when_required`. Rust fix (method names verified on docs.rs 1.152.0):
```rust
let cfg = aws_sdk_s3::config::Builder::from(&shared)
    .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
    .response_checksum_validation(ResponseChecksumValidation::WhenRequired).build();
```
(or env `AWS_REQUEST_CHECKSUM_CALCULATION=when_required`, `AWS_RESPONSE_CHECKSUM_VALIDATION=when_required`).

**Recommendation: rusty-s3 0.10.2 + the reqwest client you already have.** Sans-IO so no second HTTP/TLS stack, no crypto-provider of its own, covers list-v2/get/put/delete/delete-many/head/multipart, presigning *is* its mode (so "copy a presigned URL for the user" is free), no checksum headers so the R2 problem cannot occur, and R2 accepts SigV4 query-signed requests for all of these. Next-lightest: s3-simple; object_store with `ring` if you want the FS abstraction; avoid rust-s3 until it moves to reqwest 0.13; avoid aws-sdk-s3 for a size-sensitive TUI. Measured (same size profile): **reqwest 0.13 (ring) + rusty-s3 0.10.2 = 2.43 MB, i.e. +165 KB over the 2.27 MB reqwest baseline**; **aws-config 1.12 + aws-sdk-s3 1.152 with `["rt-tokio","rustls"]` (legacy hyper-0.14 + ring client) = 5.50 MB and a 6-minute clean build on 4 cores** (~2.4x the baseline); **aws-sdk-s3 with the modern hyper-1 client: `["rt-tokio","http-1x","default-https-client","behavior-version-latest"]` (aws-lc-rs) = 4.38 MB, 257 s clean build; same with `aws-smithy-http-client { features = ["rustls-ring"] }` and `Builder::new().tls_provider(tls::Provider::Rustls(CryptoMode::Ring)).build_https()` as `http_client` = 3.61 MB, 219 s.** Summary, same tokio runtime: reqwest+ring 2.27 MB → + rusty-s3 2.43 MB → aws-sdk-s3 ring 3.61 MB → aws-sdk-s3 aws-lc 4.38 MB → aws-sdk-s3 legacy `rustls` feature 5.50 MB (the legacy hyper-0.14 path is the worst; never use the bare `rustls` feature name on aws-sdk-s3). The SDK costs +1.3 MB (ring) to +2.1 MB (aws-lc) and 3.5–6 minutes of clean compile versus +0.16 MB and seconds for rusty-s3.

### e. OAuth (PKCE + loopback) and credential storage

**What wrangler does** (now in `packages/workers-auth` = `@cloudflare/workers-auth` 0.12.0, "Internal OAuth 2.0 + PKCE flow for Cloudflare CLIs. Not intended for external use"; `packages/wrangler/src/user/user.ts` is a shim — https://github.com/cloudflare/workers-sdk/tree/main/packages/workers-auth):
- client_id `54d11594-84e4-41aa-b438-e81b8fa78ee7` (staging `4b2ea6cc-9421-4761-874b-ce550e0e3def`, env `WRANGLER_CLIENT_ID`); auth `https://dash.cloudflare.com/oauth2/auth`, token `https://dash.cloudflare.com/oauth2/token`, revoke `https://dash.cloudflare.com/oauth2/revoke`, device `https://dash.cloudflare.com/oauth2/device/auth`; redirect `http://localhost:8976/oauth/callback` (only port known to work; a sibling `cf` CLI uses 8877); auth URL `response_type=code&client_id&redirect_uri&scope=<scopes + offline_access>&state&code_challenge&code_challenge_method=S256`; token POST form `grant_type=authorization_code&code&code_verifier&client_id&redirect_uri`; refresh `grant_type=refresh_token&refresh_token&client_id` (public client, no secret) → `{access_token, expires_in, refresh_token, scope}`; refresh token re-read from disk before each refresh (sibling processes rotate it); callback server times out after 120 s.
- Default scopes: `k2.read k2.write account:read user:read workers:write workers_kv:write workers_routes:write workers_scripts:write workers_tail:read d1:write pages:write zone:read ssl_certs:write ai:write ai-search:write ai-search:run agent-memory:write queues:write pipelines:write secrets_store:write artifacts:write flagship:write containers:write cloudchamber:write connectivity:admin email_routing:write email_sending:write browser:write challenge-widgets.write` + `offline_access` (`src/core/scopes.ts`). Note: **no DNS/zone-settings/rulesets/billing scopes** — a dashboard-replacement TUI cannot run on wrangler's grant even if it borrowed the client.
- Token file: `~/.wrangler/config/default.toml` if that legacy dir exists, else `$XDG_CONFIG_HOME/.wrangler/config/default.toml` (≈ `~/.config/.wrangler/config/default.toml`; `%APPDATA%` on Windows); fields `oauth_token`, `refresh_token`, `expiration_time` (ISO), `scopes[]` or `api_token`; written mode **0600** and re-chmodded on every write. New opt-in `wrangler login --use-keyring` (AES-256-GCM `default.enc`, key in macOS `security` / Linux `secret-tool` / Windows `@napi-rs/keyring`); plaintext file remains the default.

**Reusing wrangler's client_id**: technically works (198 GitHub code hits incl. Go/Rust/TS third-party tools), but the consent screen says "Wrangler", the package is marked not-for-external-use, and Cloudflare can re-scope it. **The premise that you cannot register your own client is now false**: self-managed OAuth clients shipped 2026-06-03 and opened to everyone 2026-06-24 (https://developers.cloudflare.com/changelog/post/2026-06-03-public-oauth-clients/, https://blog.cloudflare.com/oauth-for-all/, docs https://developers.cloudflare.com/fundamentals/oauth/create-an-oauth-client/): dashboard **Manage Account → OAuth clients** or `POST /accounts/{id}/oauth_clients`; Authorization Code only; the docs' table maps "CLI app → Authorization Code with PKCE, token_endpoint_auth_method none, PKCE S256 required"; scopes = API-token permission names from `GET /oauth/scopes`; clients start **private** (your account's members) and can be promoted to **public** after verifying the client's domain (irreversible). Register `http://localhost:<your-port>/oauth/callback`. Cloudflare's agent guidance (https://www.cloudflare.com/auth.md) lists the sanctioned paths: `wrangler login`, `CLOUDFLARE_API_TOKEN`, or your own OAuth client.

**Recommendation**: own OAuth client (private while developing, public after domain verification), PKCE S256, auth method `none`; also honour `CLOUDFLARE_API_TOKEN`, and optionally *read* wrangler's `oauth_token` as "reuse my wrangler login" but never refresh with its refresh_token (rotation races wrangler, and its scopes are too narrow anyway).

Crates: **oauth2 5.0.0** (2025-01-21; PKCE `PkceCodeChallenge::new_random_sha256()`, `CsrfToken`, refresh, revoke) still bundles **reqwest 0.12** and the 0.13 bump was closed (ramosbugs/oauth2-rs#333/#334); use `oauth2 = { version = "5", default-features = false }` and implement its `AsyncHttpClient` trait (`http::Request<Vec<u8>> -> http::Response<Vec<u8>>`) over your reqwest 0.13 client (`oauth2-reqwest 0.1.0-alpha.3` on crates.io targets 0.13 but is alpha). Hand-rolling is equally reasonable: three URLs, one form POST, `sha2`+`base64` for PKCE, `rand` for state, ~150 lines. Loopback: `tiny_http` 0.12.0 is from 2022 — use `tokio::net::TcpListener` and parse the single `GET /oauth/callback?code=…&state=…` request line, answer with a tiny HTML page or 307 to Cloudflare's consent-granted page, 120 s timeout; `open` crate for the browser, `--no-browser` for SSH.

**Storage**: `keyring` is now **4.2.0** (2026-08-29) split into `keyring-core` 1.0 + per-store crates; `keyring` 4 `default = ["v1"]` = `apple-native-keyring-store/keychain` + `windows-native-keyring-store` + `zbus-secret-service-keyring-store`; the 3.x feature names (`linux-native`, `sync-secret-service`, `linux-native-sync-persistent`, `apple-native`, `windows-native`) only exist in 3.6.3. Linux failure modes: Secret Service needs a D-Bus session bus **and** gnome-keyring/KWallet (errors on headless/SSH/containers/WSL, locked collections, KWallet UTF-8 only); `linux-keyutils-keyring-store` always works but is **memory-only** (lost at reboot/logout, persistent keyring expires in days); `dbus-secret-service-keyring-store` needs libdbus (musl pain unless `vendored`). macOS prompts per binary signature; Windows Credential Manager is reliable (2.5 KB limit). **Recommendation**: `0600` file at `$XDG_CONFIG_HOME/<app>/credentials.toml` (`%APPDATA%\<app>\` on Windows), `OpenOptions::mode(0o600)` + re-chmod on write exactly like wrangler/gh, with an opt-in `--use-keyring` that stores only a random 32-byte key via `keyring-core` + the three native stores and keeps an AES-GCM-encrypted file beside the TOML. Never make the keyring the only path.

### f. Cross-compiling static binaries in GitHub Actions

**lazymongo's approach** (https://github.com/edumntg/lazymongo, `.github/workflows/release.yml`; no local checkout here): no `cross`, no zigbuild, no `actions-rust-cross`, no `taiki-e/*` — **native runners per target** with plain `cargo build --release --target`:

```yaml
matrix.include:
  - { target: aarch64-apple-darwin,       os: macos-14 }
  - { target: x86_64-apple-darwin,        os: macos-15-intel }
  - { target: x86_64-unknown-linux-musl,  os: ubuntu-latest }
  - { target: aarch64-unknown-linux-musl, os: ubuntu-24.04-arm }
  - { target: x86_64-pc-windows-msvc,     os: windows-latest }
steps: actions/checkout@v4 → dtolnay/rust-toolchain@stable (targets: matrix.target) → Swatinem/rust-cache@v2
       → `sudo apt-get install -y musl-tools` if musl → cargo build --release --target … -p lazymongo
       → tar.gz + sha256 (unix) / Compress-Archive + Get-FileHash (windows) → upload-artifact@v4 → download-artifact@v4 (merge-multiple) → softprops/action-gh-release@v2
```
Triggered on `v*` tags; release profile `lto=true, codegen-units=1, strip=true, opt-level="z", panic="abort"`; deps `tokio 1 (rt,macros,sync,time)`, `mongodb 3` (default rustls), ratatui 0.29. The key observation: **the native `ubuntu-24.04-arm` runner removes the aarch64 cross-compilation problem**, and `musl-tools`' `musl-gcc` is enough of a C toolchain for ring (and even aws-lc-sys non-FIPS). It works unchanged for reqwest+rustls.

**The pain, with citations**: aws-lc-rs is rustls's default; non-FIPS needs only a C compiler (pre-generated bindings exist for x86_64/aarch64 musl and Apple; https://aws.github.io/aws-lc-rs/platform_support.html), NASM on Windows is avoided by `prebuilt-nasm` which rustls enables (older reports https://github.com/rustls/rustls/issues/1913, /2133). Still open/ugly: windows-gnu cross from Linux fails (https://github.com/aws/aws-lc-rs/issues/753); `cross` can't find bindgen (https://github.com/cross-rs/cross/issues/1689, open); aarch64 asm `__ARM_ARCH` undefined when cross-compiling from manylinux (https://github.com/aws/aws-lc-rs/issues/1022, open 2026-01, workaround "use zig"); musl header bug in aws-lc-sys 0.32.0 (https://github.com/aws/aws-lc-rs/issues/894). ring: RUSTSEC-2025-0007 "unmaintained" was **withdrawn** two days later (https://rustsec.org/advisories/RUSTSEC-2025-0007.html); ring's own CI tests `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl` (https://raw.githubusercontent.com/briansmith/ring/main/.github/workflows/ci.yml); its cross failures were the same `__ARM_ARCH` class (https://github.com/briansmith/ring/issues/1728, /1789; `CFLAGS_aarch64_unknown_linux_gnu="-D__ARM_ARCH=8"`) or a missing `aarch64-linux-musl-gcc` in the image (https://github.com/briansmith/ring/issues/1414, /2127; https://github.com/cross-rs/cross/discussions/578). Tooling: `cross` crates.io 0.2.5 (2023) vs active `main` (install from git); `cargo-zigbuild` 0.23.4 (Linux/macOS targets only, musl for static); `houseabsolute/actions-rust-cross` v1.0.8 wraps `cross`; `taiki-e/upload-rust-binary-action` v1.30.2 + `taiki-e/setup-cross-toolchain-action` v1.42.0 cross-compile without Docker.

**Recommendation**: (1) **ring** (`reqwest` `rustls-no-provider` + `rustls/ring`, `install_default()` in main) — builds with just a C compiler, ~0.8 MB smaller, CI-tested on both musl targets, advisory withdrawn; add a CI gate `cargo tree -e features --target $T -i aws-lc-rs` that fails if anything drags it in (tokio-tungstenite, rusty-s3, object_store `ring`, oauth2 no-default-features all stay clean; rust-s3/aws-sdk-s3 defaults do not). (2) **Copy lazymongo's native-runner matrix** (x86_64/aarch64 musl on `ubuntu-latest`/`ubuntu-24.04-arm` + `musl-tools`, `macos-14`/`macos-15-intel` (or `macos-13`), `x86_64-pc-windows-msvc` — never `-gnu`); musl targets are `crt-static` by default, verify with `file … | grep 'statically linked'`. (3) Only for targets without a native runner (armv7, riscv64, FreeBSD) use `taiki-e/setup-cross-toolchain-action` or `cargo-zigbuild`; `cross` last.

```yaml
jobs.build:
  strategy: { fail-fast: false, matrix: { include: [
    { target: x86_64-unknown-linux-musl,  os: ubuntu-latest },
    { target: aarch64-unknown-linux-musl, os: ubuntu-24.04-arm },
    { target: aarch64-apple-darwin,       os: macos-14 },
    { target: x86_64-apple-darwin,        os: macos-15-intel },
    { target: x86_64-pc-windows-msvc,     os: windows-latest } ] } }
  runs-on: ${{ matrix.os }}
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with: { targets: "${{ matrix.target }}" }
    - uses: Swatinem/rust-cache@v2
      with: { key: "${{ matrix.target }}" }
    - if: contains(matrix.target, 'musl')
      run: sudo apt-get update && sudo apt-get install -y musl-tools
    - name: Refuse aws-lc-rs in the graph
      run: if cargo tree -e features --target ${{ matrix.target }} -i aws-lc-rs >/dev/null 2>&1; then cargo tree -e features --target ${{ matrix.target }} -i aws-lc-rs; exit 1; fi
    - run: cargo build --release --locked --target ${{ matrix.target }}
    - if: contains(matrix.target, 'musl')
      run: file target/${{ matrix.target }}/release/<bin> | grep -q 'statically linked'
    # package + upload-artifact@v4 → softprops/action-gh-release@v2 as lazymongo does
```
