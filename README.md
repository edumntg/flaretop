# flaretop

> **top for Cloudflare.** A fast, keyboard-driven terminal UI for Workers,
> Pages, D1, KV, R2, Queues and Durable Objects. Browse data, tail logs,
> deploy versions and roll back, without the web dashboard. One static
> binary, ~4 MB, instant start, works over SSH.

[![CI](https://github.com/edumntg/flaretop/actions/workflows/ci.yml/badge.svg)](https://github.com/edumntg/flaretop/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange)

```
 flaretop   Acme (a1b2c3…)  profile: work  token: config                        api 12/1200 per 5m
╭ sections ──────╮┏ workers (6) ━━━━━━━━━━━━━━━━━━━━━┓╭ api ─────────────────────────────────────────────╮
│1 Workers     6 │┃name                    age  cron┃│1,204 requests/24h  2 errors  3.1ms p50 18ms p99   │
│2 Pages       2 │┃api                      2d     ⏱┃│workers.dev   https://api.acme.workers.dev        │
│3 D1          4 │┃blog                     9d      ┃│domain        https://api.example.com             │
│4 KV          3 │┃image-resizer           14d      ┃│live          7f3a9c1e… 100%  (2d)                │
│5 R2          5 │┃shop                     1d     ⏱┃│cron          */15 * * * *                        │
│6 Queues      2 │┃webhooks                 5d      ┃│bindings (5)                                      │
│7 Durable Obj…3 │┃…                                ┃│  d1          DB        c0ffee12-…                │
╰────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰──────────────────────────────────────────────────╯
 Enter detail  t tail  c code  v versions  s secret  k cron  w workers.dev  D delete  / filter  ? help
```

## Why

The dashboard is a browser tab you keep reopening. Cloudflare's new `cf` CLI
covers the whole API, one command per operation with JSON out, and it is the
right tool for scripts and agents; it is not an interactive explorer, it needs
Node, and every command costs about a second. `wrangler` deploys.

flaretop is the third thing: the `k9s`/`lazygit` experience for a Cloudflare
account. Open it, arrow through your Workers, look at a D1 table, edit a row,
page through a bucket, watch a live tail, ship a version to 10 % of traffic
and roll it back, all from one keyboard-driven screen. Every action can be
copied as its `cf` equivalent with `C`.

| | web dashboard | `cf` / `wrangler` | **flaretop** |
|---|---|---|---|
| Browse data and logs interactively | ✅ | ❌ | ✅ |
| Works over SSH / in tmux | ❌ | ✅ | ✅ |
| Startup | seconds | ~1 s (Node) | instant |
| Footprint | a browser | 140 MB per command | **~4 MB binary, ~10 MB RAM** |
| Scriptable | ❌ | ✅ | copy-as-`cf` |

## What it does

| Section | Read | Write |
|---|---|---|
| **Workers** | list, detail (24 h requests/errors/CPU, bindings, secrets, cron, domains, versions, deployments), deployed code | set/delete secrets, cron triggers, workers.dev toggle, deploy a version (100 % or gradual), rollback, delete |
| **Live tail** | WebSocket tail with pause, select, raw event, copy, client-side filter | — |
| **Pages** | projects (repo, branch, domains, latest build), deployments with status/branch/commit, build logs | roll production back to a deployment, retry a build, delete project |
| **D1** | databases, tables, SQL console, table grid with horizontal scroll, row as JSON | edit / insert / delete rows (generated SQL shown before running), any DDL/DML with confirmation, create / delete database, Time Travel bookmark |
| **KV** | namespaces, keys (cursor paging, prefix filter), value + metadata viewer | put / edit value with TTL, edit metadata, delete key, bulk delete loaded keys, create / delete namespace |
| **R2** | buckets (size, object count, public domain), object browser with folders | upload (≤ 300 MB), download, delete object, toggle r2.dev public access, create / delete bucket |
| **Queues** | queues, producers, consumers, settings | push message, pull/ack (http_pull consumers), pause / resume, purge, create / delete |
| **Durable Objects** | namespaces, object ids, has-data flag | — (the API exposes nothing else) |

Every destructive action asks for confirmation; dangerous ones require typing
the resource name. `--readonly` blocks every write at the I/O layer. The
header shows how much of the 1,200 requests / 5 min API budget the session
has used. Several accounts and tokens are handled with profiles (`P`).

## Install

**Prebuilt binaries** for macOS (Apple Silicon and Intel), Linux (static
musl, x86_64 and arm64) and Windows are on the
[releases page](https://github.com/edumntg/flaretop/releases):

```sh
tar xzf flaretop-*-<your-target>.tar.gz
mv flaretop-*/flaretop ~/.local/bin/      # or anywhere on your PATH
```

**From source** (Rust 1.88+):

```sh
cargo install --git https://github.com/edumntg/flaretop flaretop
# or, from a clone, at the repository root:
cargo install --path flaretop
```

## Quick start

```sh
# 1. create an API token (see "The token" below), then either
export CLOUDFLARE_API_TOKEN=cfut_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
#    or save it once (hidden input, verified against the API, stored with mode 0600):
flaretop --login

# 2. run
flaretop
```

If the token can see more than one account you get a picker; the choice is
remembered in `~/.config/flaretop/config.toml`. Skip the picker with
`flaretop --account <ACCOUNT_ID>` or `export CLOUDFLARE_ACCOUNT_ID=...`.

## The token

flaretop talks to `api.cloudflare.com` with a Bearer **API token**. The
legacy Global API Key is not supported. Both token kinds work:

- a **user token** (dashboard → My Profile → API Tokens), which can list all
  the accounts you belong to, or
- an **account-owned token** (dashboard → Manage Account → Account API
  Tokens); these cannot list accounts, so start with `--account ID`.

### Create it

1. Open https://dash.cloudflare.com/profile/api-tokens and click
   **Create Token** → **Create Custom Token**.
2. Give it a name (for example `flaretop`).
3. Add these permissions. Each row is *Scope / Group / Level*; use
   **Read** instead of **Edit** on any product you only want to browse.

   | Scope | Permission group | Level | Unlocks |
   |---|---|---|---|
   | Account | Workers Scripts | Edit | Workers list, detail, secrets, cron, workers.dev, versions, deployments, delete; Durable Object inventory |
   | Account | Workers Tail | Read | live tail (`t`) |
   | Account | Cloudflare Pages | Edit | Pages projects, deployments, rollback, retry, logs |
   | Account | D1 | Edit | D1 databases, tables, SQL console, row edits, Time Travel |
   | Account | Workers KV Storage | Edit | KV namespaces, keys, values |
   | Account | Workers R2 Storage | Edit | R2 buckets, objects, r2.dev toggle |
   | Account | Queues | Edit | queues, push, pull/ack, pause, purge |
   | Account | Account Analytics | Read | the 24 h request/error/CPU numbers, R2 sizes, queue backlog (GraphQL) |
   | Account | Account Settings | Read | lets the account picker list your accounts |

4. Under **Account Resources** choose *Include → the account(s)* you want to
   manage. Leave **Client IP Address Filtering** empty unless you always run
   from one IP. A **TTL** is optional.
5. **Continue to summary** → **Create Token** → copy the value. It is shown
   once; new tokens start with `cfut_` (user) or `cfat_` (account).

Check it from a shell before using it:

```sh
curl -s -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
  https://api.cloudflare.com/client/v4/user/tokens/verify
# → {"result":{"id":"…","status":"active"},"success":true,…}
```

### What a missing permission looks like

Nothing crashes. The section shows *token lacks permission for this
product* and the operations log (`L`) keeps the exact error. Add the
permission in the dashboard (tokens can be edited in place) and press `r`.

### Where the token lives

In precedence order:

1. `CLOUDFLARE_API_TOKEN` in the environment. Put the `export` line in your
   shell profile (`~/.zshrc`, `~/.bashrc`) or use a tool like `direnv`.
2. `~/.config/flaretop/credentials.toml`, written by `flaretop --login` with
   mode 0600. Forget it with `flaretop --logout`.
3. With `--cli-login` (or `use_cli_logins = true` in the config): the OAuth
   token stored by `cf auth login` (`~/.config/cloudflare/config/default.json`,
   `~/Library/Preferences/cloudflare/config/default.json` on macOS) or by
   `wrangler login`. Those access tokens live about an hour and the CLIs only
   refresh them when they run a command, so `cf auth login` can say *You are
   already logged in* while the stored token is already dead. When the API
   rejects it, flaretop runs `cf auth whoami` (or `wrangler whoami`) to make
   the CLI refresh its file and retries once; if that fails, run
   `cf auth login --force` and start flaretop again. An API token (options 1
   and 2) does not expire and is the recommended setup.
4. `--token <value>` on the command line, for one-off use (it ends up in
   your shell history; prefer the options above).

The header shows which source is in use (`token: env`, `token: config`,
`token: cf login`).

### Several tokens or accounts (profiles)

A profile is a saved token plus, optionally, the account it opens. Use one
per client, team or environment:

```sh
flaretop --login --profile work        # paste token; picks the account if the token sees several
flaretop --login --profile client-acme --account 9f8e7d…
flaretop --profiles                    # list them
flaretop -p client-acme                # start with one
flaretop --logout --profile work       # forget one
```

Without `-p`, flaretop uses the last profile you opened (or the only one).
Inside the app, `P` opens the profile picker and switches token, account and
all panes without restarting; `A` switches between the accounts the current
token can see. Profiles live in `~/.config/flaretop/credentials.toml` (mode
0600) as `[profiles.<name>]` tables.

## Running

```sh
flaretop                      # last account, or a picker
flaretop --account 21e9f4…    # a specific account
flaretop --readonly           # browse production with every write blocked
flaretop --theme dark         # cloudflare (default) | cloudflare-light | dark | light |
                              # claude-dark | claude-light | termius | high-contrast
flaretop --no-anim            # skip the start-up animation (also FLARETOP_NO_ANIM=1)
flaretop --help
```

Terminal requirements: any terminal with 256 colors; mouse and truecolor are
optional (the default theme uses truecolor, `dark` and `light` use the
terminal's own ANSI palette). Works inside tmux and over SSH. Pane sizes and
the theme are remembered in `~/.config/flaretop/config.toml`; the theme can
also be switched live from the palette.

## Keys

`?` inside the app shows this list. Keys act on the selected row of the
focused pane; section keys work from the list and the detail pane alike.

**Global**

| Key | Action |
|---|---|
| `1`–`7` | jump to Workers, Pages, D1, KV, R2, Queues, Durable Objects |
| `Tab` / `Shift-Tab` | cycle sidebar → list → detail |
| `h` `l` / `←` `→` | move between panes (in a D1 grid: scroll columns) |
| `j` `k` / `↑` `↓`, `g` `G`, `PgUp` `PgDn` | move |
| `Enter` | open the selected thing |
| `/` | filter the list (`Esc` clears) |
| `r` | refresh |
| `:` or `Ctrl-P` | command palette (every action, fuzzy search) |
| `C` | copy the equivalent `cf` command |
| `L` | operations log |
| `A` / `P` | switch account / switch profile |
| `[` `]` / `{` `}` | narrower or wider list pane / sidebar (or drag a divider with the mouse) · `=` reset |
| `?` | help · `q` quit |

**Workers**

| Key | Action |
|---|---|
| `t` | live tail |
| `c` | view the deployed code (all modules) |
| `v` | versions list · `Enter` deploy 100 % · `%` gradual rollout |
| `s` / `x` | set / delete a secret |
| `k` | edit cron triggers |
| `w` | toggle workers.dev |
| `y` | copy the workers.dev URL |
| `D` | delete the worker (type its name) |

**Live tail**

| Key | Action |
|---|---|
| `Space` / `p` | pause (events buffer) |
| `j` `k` | select an event · `G` follow again |
| `Enter` / `y` | raw event / copy it |
| `f` | client-side filter · `c` clear · `Esc` stop |

**Pages**

| Key | Action |
|---|---|
| `Enter` | deployment details |
| `l` | build log |
| `R` | roll production back to the selected deployment |
| `b` | retry the build |
| `y` | copy deployment URL · `D` delete project |

**D1**

| Key | Action |
|---|---|
| `Enter` | open table (first 200 rows) · in a grid: row as JSON |
| `s` | SQL console (`Ctrl-S` runs, `Ctrl-U` clears; writes ask first) |
| `e` / `i` / `d` | edit / insert / delete the selected row (SQL shown before running) |
| `h` `l` | scroll columns · `Esc` back to tables |
| `n` / `D` | new database / delete database (type its name) |
| `b` | Time Travel bookmark |

**KV**

| Key | Action |
|---|---|
| `Enter` | view value and metadata |
| `e` / `m` | edit value (asks TTL) / edit metadata |
| `n` / `d` | new key / delete key |
| `p` | prefix filter · `G` load next page |
| `D` | delete every loaded key (type the count) |
| `N` / `X` | new namespace / delete namespace (type its title) |

**R2**

| Key | Action |
|---|---|
| `Enter` | open folder / view object · `Backspace` up |
| `s` / `u` | download to a path / upload a local file (≤ 300 MB) |
| `d` | delete object |
| `p` | toggle r2.dev public access |
| `G` | load next page |
| `N` / `X` | new bucket / delete bucket (type its name) |

**Queues**

| Key | Action |
|---|---|
| `m` | push a message (JSON or text) |
| `l` | pull a batch (needs an `http_pull` consumer; leases 30 s) |
| `a` / `A` | ack selected / ack all pulled |
| `p` | pause / resume delivery |
| `P` | purge (type the name) · `n` new · `D` delete |

**Durable Objects**: `Enter` or `y` copies an object id, `G` loads more.
The API exposes nothing else.

**Editors and viewers**: `Ctrl-S` saves, `Esc` cancels, `Ctrl-A`/`Ctrl-E`
line start/end, `Ctrl-U` clear, `y` copies a viewer's contents, `Tab` jumps
between modules in the code viewer.

## Limits worth knowing

These are Cloudflare API limits, not flaretop's:

- **Queues** have no non-consuming peek; pulling leases messages for 30 s and
  only works with an `http_pull` consumer.
- **Durable Objects** expose namespaces and object ids only; storage, alarms
  and deletion are not in the API.
- **Secrets** are write-only; you see names, never values.
- **R2 objects over 300 MB** need the S3 API (not wired yet).
- **D1** has no table-list endpoint; flaretop queries `sqlite_master`.
- The API allows **1,200 requests per 5 minutes** per user, shared with the
  dashboard. flaretop loads lazily and shows the running count.

## Roadmap

- Zones: DNS records table, SSL mode, cache purge, zone settings
- Account: API tokens, members, audit log, notification policies
- Workers Logs (persistent) query builder, metrics charts
- Workflows, Vectorize, Hyperdrive, D1 export/import, R2 multipart via S3
- Homebrew tap and `curl | sh` installer

## Layout

```
flaretop-core/   API client, auth, product modules, I/O actor (no TUI dependency)
flaretop/        the binary (ratatui)
docs/research/   what the Cloudflare API can and cannot do, verified 2026-10-05
PLAN.md          the phased plan and the decisions behind it
```

Contributions welcome: open an issue with what you are trying to do from the
terminal and cannot. Built with [ratatui](https://ratatui.rs), in the spirit
of [lazymongo](https://github.com/edumntg/lazymongo). Not affiliated with
Cloudflare.

## License

MIT
