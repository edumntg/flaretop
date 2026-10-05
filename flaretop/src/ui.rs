//! Drawing. Layout: header, [sidebar | list | detail], footer; modals and
//! the tail screen draw on top.

use flaretop_core::actor::bindings_rows;
use flaretop_core::products::queues;
use flaretop_core::{age, human_bytes, short_time};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, D1View, Focus, R2Entry, SECTIONS, Section};
use crate::json_view::{cell_style, cell_text};
use crate::modal::Modal;
use crate::theme;
use crate::util::{clock_from_ms, fit, one_line, pad};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(f, app, header);
    if app.tail.is_some() {
        draw_tail(f, app, body);
    } else {
        let [sidebar, rest] =
            Layout::horizontal([Constraint::Length(app.sidebar_w), Constraint::Min(20)])
                .areas(body);
        app.rects.body = body;
        let (list_w, detail_w) = (app.list_pct, 100 - app.list_pct);
        let [list, detail] = Layout::horizontal([
            Constraint::Percentage(list_w),
            Constraint::Percentage(detail_w),
        ])
        .areas(rest);
        app.rects.sidebar = sidebar;
        app.rects.list = list;
        app.rects.detail = detail;
        draw_sidebar(f, app, sidebar);
        draw_list(f, app, list);
        draw_detail(f, app, detail);
    }
    draw_footer(f, app, footer);
    draw_modal(f, app, area);
    draw_toasts(f, app, area);
}

fn block(title: impl Into<String>, focused: bool) -> Block<'static> {
    let color = if focused {
        theme::accent()
    } else {
        theme::border_dim()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::new().fg(color))
        .title(Line::from(Span::styled(
            format!(" {} ", title.into()),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        )))
}

fn dim() -> Style {
    Style::new().fg(theme::dim())
}
fn text() -> Style {
    Style::new().fg(theme::text())
}
fn accent() -> Style {
    Style::new().fg(theme::accent())
}
fn sel_style(focused: bool) -> Style {
    Style::new()
        .bg(if focused {
            theme::sel_bg()
        } else {
            theme::sel_bg_dim()
        })
        .fg(theme::text())
        .add_modifier(Modifier::BOLD)
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(
            " flaretop ",
            Style::new()
                .fg(theme::badge_fg())
                .bg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(app.account_label(), text()),
        Span::raw("  "),
        Span::styled(
            app.profile
                .as_ref()
                .map(|p| format!("profile: {p}  "))
                .unwrap_or_default(),
            dim(),
        ),
        Span::styled(
            match &app.cred_expires {
                Some(e) => format!(
                    "token: {} (expires {})",
                    app.cred_source.label(),
                    short_time(e)
                ),
                None => format!("token: {}", app.cred_source.label()),
            },
            dim(),
        ),
    ];
    if app.read_only {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            " READ-ONLY ",
            Style::new().fg(theme::badge_fg()).bg(theme::warn()),
        ));
    }
    let used = app.client.budget_used();
    let budget = format!("api {used}/1200 per 5m");
    let budget_style = if used > 900 {
        Style::new().fg(theme::error())
    } else if used > 600 {
        Style::new().fg(theme::warn())
    } else {
        dim()
    };
    let right: Vec<Span> = vec![
        if app.inflight > 0 {
            Span::styled(format!("{} {} ", app.spinner(), app.inflight), accent())
        } else {
            Span::raw("")
        },
        Span::styled(budget, budget_style),
        Span::raw(" "),
    ];
    let right_w: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let left_w: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let gap = (area.width as usize).saturating_sub(left_w + right_w);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let b = block("sections", focused);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let mut lines = Vec::new();
    for (i, s) in SECTIONS.iter().enumerate() {
        let count = match s {
            Section::Workers => app
                .workers
                .list
                .loaded
                .then_some(app.workers.list.items.len()),
            Section::Pages => app.pages.list.loaded.then_some(app.pages.list.items.len()),
            Section::D1 => app.d1.list.loaded.then_some(app.d1.list.items.len()),
            Section::Kv => app.kv.list.loaded.then_some(app.kv.list.items.len()),
            Section::R2 => app.r2.list.loaded.then_some(app.r2.list.items.len()),
            Section::Queues => app
                .queues
                .list
                .loaded
                .then_some(app.queues.list.items.len()),
            Section::DurableObjects => app.dos.list.loaded.then_some(app.dos.list.items.len()),
        };
        let label = format!("{} {}", i + 1, s.label());
        let count = count.map(|c| c.to_string()).unwrap_or_default();
        let w = inner.width as usize;
        let line = format!(
            "{}{}",
            pad(&label, w.saturating_sub(count.len() + 1)),
            count
        );
        let style = if *s == app.section {
            sel_style(focused)
        } else {
            text()
        };
        lines.push(Line::from(Span::styled(pad(&line, w), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Render a list pane with a header row and one highlighted row; keeps the
/// selection visible by adjusting `scroll`.
struct Rows {
    header: String,
    rows: Vec<(String, Style)>,
}

fn render_rows(
    f: &mut Frame,
    area: Rect,
    rows: &Rows,
    selected: usize,
    scroll: &mut usize,
    focused: bool,
) {
    let h = area.height as usize;
    if h == 0 {
        return;
    }
    let body_h = h.saturating_sub(1);
    if selected < *scroll {
        *scroll = selected;
    } else if body_h > 0 && selected >= *scroll + body_h {
        *scroll = selected + 1 - body_h;
    }
    let w = area.width as usize;
    let mut lines = vec![Line::from(Span::styled(
        pad(&rows.header, w),
        dim().add_modifier(Modifier::UNDERLINED),
    ))];
    for (i, (txt, style)) in rows.rows.iter().enumerate().skip(*scroll).take(body_h) {
        let st = if i == selected {
            sel_style(focused)
        } else {
            *style
        };
        lines.push(Line::from(Span::styled(pad(txt, w), st)));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::List;
    let (title, loading) = match app.section {
        Section::Workers => (
            format!("workers ({})", app.workers.list.len()),
            app.workers.list.loading,
        ),
        Section::Pages => (
            format!("pages projects ({})", app.pages.list.len()),
            app.pages.list.loading,
        ),
        Section::D1 => (
            format!("d1 databases ({})", app.d1.list.len()),
            app.d1.list.loading,
        ),
        Section::Kv => (
            format!("kv namespaces ({})", app.kv.list.len()),
            app.kv.list.loading,
        ),
        Section::R2 => (
            format!("r2 buckets ({})", app.r2.list.len()),
            app.r2.list.loading,
        ),
        Section::Queues => (
            format!("queues ({})", app.queues.list.len()),
            app.queues.list.loading,
        ),
        Section::DurableObjects => (
            format!("durable object namespaces ({})", app.dos.list.len()),
            app.dos.list.loading,
        ),
    };
    let title = if loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let filter = crate::keys::current_filter(app);
    let mut b = block(title, focused);
    if app.filter_mode || !filter.is_empty() {
        b = b.title_bottom(Line::from(Span::styled(
            format!(" /{filter}{} ", if app.filter_mode { "▏" } else { "" }),
            accent(),
        )));
    }
    let inner = b.inner(area);
    app.rects.list_inner = inner;
    f.render_widget(b, area);
    let w = inner.width as usize;
    let rows = match app.section {
        Section::Workers => {
            let l = &app.workers.list;
            Rows {
                header: format!(
                    "{} {:>4} {:>5}",
                    pad("name", w.saturating_sub(11)),
                    "age",
                    "cron"
                ),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let s = &l.items[i];
                        let has_cron = s.handlers.iter().any(|h| h == "scheduled");
                        (
                            format!(
                                "{} {:>4} {:>5}",
                                pad(&s.id, w.saturating_sub(11)),
                                s.modified_on.as_deref().map(age).unwrap_or_default(),
                                if has_cron { "⏱" } else { "" }
                            ),
                            text(),
                        )
                    })
                    .collect(),
            }
        }
        Section::Pages => {
            let l = &app.pages.list;
            Rows {
                header: format!("{} {:>4}", pad("name", w.saturating_sub(5)), "age"),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let p = &l.items[i];
                        (
                            format!(
                                "{} {:>4}",
                                pad(&p.name, w.saturating_sub(5)),
                                p.latest_modified().map(age).unwrap_or_default()
                            ),
                            text(),
                        )
                    })
                    .collect(),
            }
        }
        Section::D1 => {
            let l = &app.d1.list;
            Rows {
                header: format!("{} {:>8}", pad("name", w.saturating_sub(9)), "size"),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let d = &l.items[i];
                        (
                            format!(
                                "{} {:>8}",
                                pad(&d.name, w.saturating_sub(9)),
                                d.file_size.map(human_bytes).unwrap_or_default()
                            ),
                            text(),
                        )
                    })
                    .collect(),
            }
        }
        Section::Kv => {
            let l = &app.kv.list;
            Rows {
                header: pad("title", w),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| (pad(&l.items[i].title, w), text()))
                    .collect(),
            }
        }
        Section::R2 => {
            let l = &app.r2.list;
            Rows {
                header: format!("{} {:>5}", pad("name", w.saturating_sub(6)), "loc"),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let b = &l.items[i];
                        (
                            format!(
                                "{} {:>5}",
                                pad(&b.name, w.saturating_sub(6)),
                                b.location.clone().unwrap_or_default()
                            ),
                            text(),
                        )
                    })
                    .collect(),
            }
        }
        Section::Queues => {
            let l = &app.queues.list;
            Rows {
                header: format!(
                    "{} {:>3} {:>3} {:>6}",
                    pad("name", w.saturating_sub(15)),
                    "prd",
                    "con",
                    "state"
                ),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let q = &l.items[i];
                        (
                            format!(
                                "{} {:>3} {:>3} {:>6}",
                                pad(&q.queue_name, w.saturating_sub(15)),
                                q.producers.len(),
                                q.consumers.len(),
                                if q.paused() { "paused" } else { "ok" }
                            ),
                            if q.paused() {
                                Style::new().fg(theme::warn())
                            } else {
                                text()
                            },
                        )
                    })
                    .collect(),
            }
        }
        Section::DurableObjects => {
            let l = &app.dos.list;
            Rows {
                header: format!("{} {}", pad("worker", w / 2), "class"),
                rows: l
                    .visible
                    .iter()
                    .map(|&i| {
                        let n = &l.items[i];
                        (
                            format!(
                                "{} {}{}",
                                pad(n.script.as_deref().unwrap_or("-"), w / 2),
                                n.class.as_deref().unwrap_or("-"),
                                if n.use_sqlite.unwrap_or(false) {
                                    " ▪sql"
                                } else {
                                    ""
                                }
                            ),
                            text(),
                        )
                    })
                    .collect(),
            }
        }
    };
    let (selected, scroll) = match app.section {
        Section::Workers => (app.workers.list.selected, &mut app.workers.list.scroll),
        Section::Pages => (app.pages.list.selected, &mut app.pages.list.scroll),
        Section::D1 => (app.d1.list.selected, &mut app.d1.list.scroll),
        Section::Kv => (app.kv.list.selected, &mut app.kv.list.scroll),
        Section::R2 => (app.r2.list.selected, &mut app.r2.list.scroll),
        Section::Queues => (app.queues.list.selected, &mut app.queues.list.scroll),
        Section::DurableObjects => (app.dos.list.selected, &mut app.dos.list.scroll),
    };
    let list_error = match app.section {
        Section::Workers => app.workers.list.error.clone(),
        Section::Pages => app.pages.list.error.clone(),
        Section::D1 => app.d1.list.error.clone(),
        Section::Kv => app.kv.list.error.clone(),
        Section::R2 => app.r2.list.error.clone(),
        Section::Queues => app.queues.list.error.clone(),
        Section::DurableObjects => app.dos.list.error.clone(),
    };
    if rows.rows.is_empty()
        && let Some(err) = list_error
    {
        f.render_widget(
            Paragraph::new(Span::styled(err, Style::new().fg(theme::error())))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    if rows.rows.is_empty() {
        let msg = if loading {
            "loading…"
        } else if filter.is_empty() {
            "nothing here"
        } else {
            "no match"
        };
        f.render_widget(Paragraph::new(Line::from(Span::styled(msg, dim()))), inner);
        return;
    }
    render_rows(f, inner, &rows, selected, scroll, focused);
}

fn draw_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Detail;
    match app.section {
        Section::Workers => draw_worker_detail(f, app, area, focused),
        Section::Pages => draw_pages_detail(f, app, area, focused),
        Section::D1 => draw_d1_detail(f, app, area, focused),
        Section::Kv => draw_kv_detail(f, app, area, focused),
        Section::R2 => draw_r2_detail(f, app, area, focused),
        Section::Queues => draw_queue_detail(f, app, area, focused),
        Section::DurableObjects => draw_do_detail(f, app, area, focused),
    }
}

fn kv_line(k: &str, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:<14}"), dim()),
        Span::styled(v.into(), text()),
    ])
}

fn section_title(t: &str) -> Line<'static> {
    Line::from(Span::styled(
        t.to_string(),
        accent().add_modifier(Modifier::BOLD),
    ))
}

fn draw_worker_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let name = app.workers.detail_for.clone();
    let title = if app.workers.versions_mode {
        format!("{name} · versions")
    } else if name.is_empty() {
        "worker".to_string()
    } else {
        name.clone()
    };
    let title = if app.workers.detail_loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let b = block(title, focused);
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    let Some(d) = &app.workers.detail else {
        f.render_widget(
            Paragraph::new(Span::styled(
                if app.workers.detail_loading {
                    "loading…"
                } else {
                    "select a worker"
                },
                dim(),
            )),
            inner,
        );
        return;
    };
    let w = inner.width as usize;
    if app.workers.versions_mode {
        let live: std::collections::HashMap<&str, f64> = d
            .deployments
            .first()
            .map(|dep| {
                dep.versions
                    .iter()
                    .map(|v| (v.version_id.as_str(), v.percentage))
                    .collect()
            })
            .unwrap_or_default();
        let rows = Rows {
            header: format!(
                "{:>4} {:<9} {:<16} {:>5} {:<14} {}",
                "#", "id", "created", "live", "by", "message"
            ),
            rows: d
                .versions
                .iter()
                .map(|v| {
                    let pct = live
                        .get(v.id.as_str())
                        .map(|p| format!("{p:.0}%"))
                        .unwrap_or_default();
                    let msg = v.message().unwrap_or("").to_string();
                    let txt = format!(
                        "{:>4} {:<9} {:<16} {:>5} {:<14} {}",
                        v.number.map(|n| n.to_string()).unwrap_or_default(),
                        &v.id[..8.min(v.id.len())],
                        v.created_on().map(short_time).unwrap_or_default(),
                        pct,
                        fit(v.triggered_by().unwrap_or(""), 14),
                        fit(&msg, w.saturating_sub(54)),
                    );
                    (
                        txt,
                        if pct.is_empty() {
                            text()
                        } else {
                            Style::new().fg(theme::ok())
                        },
                    )
                })
                .collect(),
        };
        let sel = app.workers.versions.sel;
        render_rows(
            f,
            inner,
            &rows,
            sel,
            &mut app.workers.versions.scroll,
            focused,
        );
        return;
    }
    let s = app.workers.list.current().cloned().unwrap_or_default();
    let mut lines: Vec<Line> = Vec::new();
    if let Some(st) = &d.stats {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}", st.requests),
                text().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" requests/24h  ", dim()),
            Span::styled(
                format!("{}", st.errors),
                if st.errors > 0 {
                    Style::new().fg(theme::error())
                } else {
                    text()
                },
            ),
            Span::styled(" errors  ", dim()),
            Span::styled(format!("{:.1}", st.cpu_p50_us / 1000.0), text()),
            Span::styled("ms p50 ", dim()),
            Span::styled(format!("{:.1}", st.cpu_p99_us / 1000.0), text()),
            Span::styled("ms p99 cpu", dim()),
        ]));
    }
    lines.push(kv_line(
        "modified",
        s.modified_on.as_deref().map(short_time).unwrap_or_default(),
    ));
    lines.push(kv_line(
        "compat",
        format!(
            "{} {}",
            s.compatibility_date.clone().unwrap_or_default(),
            s.compatibility_flags.join(",")
        ),
    ));
    lines.push(kv_line("handlers", s.handlers.join(", ")));
    lines.push(kv_line(
        "deployed from",
        s.last_deployed_from.clone().unwrap_or_default(),
    ));
    let sub = match (&d.subdomain, &d.account_subdomain) {
        (Some(sd), Some(acct)) if sd.enabled => format!("https://{}.{acct}.workers.dev", d.name),
        (Some(_), _) => "disabled".into(),
        _ => "?".into(),
    };
    lines.push(kv_line("workers.dev", sub));
    for dom in &d.domains {
        lines.push(kv_line("domain", format!("https://{}", dom.hostname)));
    }
    if let Some(dep) = d.deployments.first() {
        let vs: Vec<String> = dep
            .versions
            .iter()
            .map(|v| {
                format!(
                    "{}… {:.0}%",
                    &v.version_id[..8.min(v.version_id.len())],
                    v.percentage
                )
            })
            .collect();
        lines.push(kv_line(
            "live",
            format!(
                "{}  ({})",
                vs.join(" + "),
                dep.created_on.as_deref().map(age).unwrap_or_default()
            ),
        ));
    }
    if !d.schedules.is_empty() {
        lines.push(kv_line(
            "cron",
            d.schedules
                .iter()
                .map(|s| s.cron.clone())
                .collect::<Vec<_>>()
                .join("  |  "),
        ));
    }
    if let Some(obs) = s
        .observability
        .as_ref()
        .and_then(|o| o.get("enabled"))
        .and_then(|v| v.as_bool())
    {
        lines.push(kv_line(
            "logs",
            if obs {
                "workers logs enabled"
            } else {
                "workers logs off"
            },
        ));
    }
    lines.push(Line::raw(""));
    if let Some(settings) = &d.settings {
        let rows = bindings_rows(settings);
        lines.push(section_title(&format!("bindings ({})", rows.len())));
        for (ty, name, target) in rows {
            let target = if ty == "secret_text" {
                "••••••".to_string()
            } else {
                target
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<24}", fit(&ty, 24)), dim()),
                Span::styled(
                    format!("{:<22}", fit(&name, 22)),
                    Style::new().fg(theme::key()),
                ),
                Span::styled(fit(&target, w.saturating_sub(50)), text()),
            ]));
        }
        lines.push(Line::raw(""));
    }
    lines.push(section_title(&format!("secrets ({})", d.secrets.len())));
    if !d.secrets.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                d.secrets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join("  ")
            ),
            text(),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(section_title(&format!(
        "deployments ({})",
        d.deployments.len()
    )));
    for dep in d.deployments.iter().take(8) {
        let vs: Vec<String> = dep
            .versions
            .iter()
            .map(|v| {
                format!(
                    "{}…{:.0}%",
                    &v.version_id[..8.min(v.version_id.len())],
                    v.percentage
                )
            })
            .collect();
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  {:<17}",
                    dep.created_on
                        .as_deref()
                        .map(short_time)
                        .unwrap_or_default()
                ),
                dim(),
            ),
            Span::styled(format!("{:<22}", fit(&vs.join("+"), 22)), text()),
            Span::styled(
                fit(
                    dep.message().or(dep.triggered_by()).unwrap_or(""),
                    w.saturating_sub(42),
                ),
                dim(),
            ),
        ]));
    }
    for e in &d.errors {
        lines.push(Line::from(Span::styled(
            format!("! {e}"),
            Style::new().fg(theme::warn()),
        )));
    }
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.workers.scroll = app.workers.scroll.min(max_scroll);
    f.render_widget(
        Paragraph::new(lines).scroll((app.workers.scroll as u16, 0)),
        inner,
    );
}

fn draw_pages_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let p = app.pages.list.current().cloned();
    let title = match &p {
        Some(p) => format!("{} · deployments ({})", p.name, app.pages.deployments.len()),
        None => "deployments".into(),
    };
    let title = if app.pages.loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let b = block(title, focused).title_bottom(Line::from(Span::styled(
        " Enter details · l log · R rollback · b retry · y url ",
        dim(),
    )));
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    let Some(p) = p else {
        f.render_widget(
            Paragraph::new(Span::styled("select a project", dim())),
            inner,
        );
        return;
    };
    let mut info = vec![
        kv_line(
            "url",
            p.subdomain
                .clone()
                .map(|s| format!("https://{s}"))
                .unwrap_or_default(),
        ),
        kv_line("domains", p.domains.join("  ")),
        kv_line(
            "source",
            format!(
                "{}  branch {}",
                p.repo().unwrap_or_else(|| "direct upload".into()),
                p.production_branch.clone().unwrap_or_default()
            ),
        ),
    ];
    if let Some(st) = p.latest_status() {
        info.push(kv_line("latest", st));
    }
    info.push(Line::raw(""));
    let head_h = (info.len() as u16).min(inner.height);
    let [head, list] =
        Layout::vertical([Constraint::Length(head_h), Constraint::Min(0)]).areas(inner);
    f.render_widget(Paragraph::new(info), head);
    let w = list.width as usize;
    let rows = Rows {
        header: format!(
            "{:<8} {:<11} {:<16} {:<18} {}",
            "id", "env", "status", "branch", "message"
        ),
        rows: app
            .pages
            .deployments
            .iter()
            .map(|d| {
                let status = d.status();
                let style = if d.succeeded() {
                    text()
                } else if status.contains("failure") {
                    Style::new().fg(theme::error())
                } else {
                    Style::new().fg(theme::warn())
                };
                (
                    format!(
                        "{:<8} {:<11} {:<16} {:<18} {}",
                        d.short_id.clone().unwrap_or_default(),
                        fit(d.environment.as_deref().unwrap_or(""), 11),
                        fit(&status, 16),
                        fit(d.branch().unwrap_or(""), 18),
                        fit(
                            &format!(
                                "{}  {}",
                                d.commit_message().unwrap_or(""),
                                d.created_on.as_deref().map(age).unwrap_or_default()
                            ),
                            w.saturating_sub(58)
                        ),
                    ),
                    style,
                )
            })
            .collect(),
    };
    if rows.rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                if app.pages.loading {
                    "loading…"
                } else {
                    "no deployments"
                },
                dim(),
            )),
            list,
        );
        return;
    }
    let sel = app.pages.dep.sel;
    render_rows(f, list, &rows, sel, &mut app.pages.dep.scroll, focused);
}

fn draw_d1_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let db = app.d1.list.current().cloned();
    let Some(db) = db else {
        let b = block("d1", focused);
        app.rects.detail_inner = b.inner(area);
        f.render_widget(b, area);
        return;
    };
    match app.d1.view {
        D1View::Tables => {
            let title = format!("{} · tables ({})", db.name, app.d1.tables.len());
            let title = if app.d1.tables_loading {
                format!("{title} {}", app.spinner())
            } else {
                title
            };
            let b = block(title, focused).title_bottom(Line::from(Span::styled(
                " Enter open · s sql · b bookmark ",
                dim(),
            )));
            let inner = b.inner(area);
            app.rects.detail_inner = inner;
            f.render_widget(b, area);
            let [info, list] =
                Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
            let region = db.running_in_region.clone().unwrap_or_default();
            let repl = db
                .read_replication
                .as_ref()
                .and_then(|r| r.get("mode"))
                .and_then(|m| m.as_str())
                .unwrap_or("-")
                .to_string();
            f.render_widget(
                Paragraph::new(vec![
                    kv_line(
                        "size",
                        format!(
                            "{}  region {region}  replication {repl}",
                            db.file_size.map(human_bytes).unwrap_or_default()
                        ),
                    ),
                    kv_line("id", db.uuid.clone()),
                ]),
                info,
            );
            let w = list.width as usize;
            let rows = Rows {
                header: format!("{} {:>5}", pad("name", w.saturating_sub(6)), "type"),
                rows: app
                    .d1
                    .tables
                    .iter()
                    .map(|t| {
                        (
                            format!("{} {:>5}", pad(&t.name, w.saturating_sub(6)), t.kind),
                            text(),
                        )
                    })
                    .collect(),
            };
            if rows.rows.is_empty() {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        if app.d1.tables_loading {
                            "loading…"
                        } else {
                            "no tables"
                        },
                        dim(),
                    )),
                    list,
                );
                return;
            }
            let sel = app.d1.table.sel;
            render_rows(f, list, &rows, sel, &mut app.d1.table.scroll, focused);
        }
        D1View::Grid => {
            let title = match &app.d1.current_table {
                Some(t) => format!("{} · {t}", db.name),
                None => format!("{} · query", db.name),
            };
            let title = if app.d1.grid_loading {
                format!("{title} {}", app.spinner())
            } else {
                title
            };
            let b = block(title, focused).title_bottom(Line::from(Span::styled(
                " Esc tables · s sql · e edit · i insert · d delete · h/l columns · Enter row ",
                dim(),
            )));
            let inner = b.inner(area);
            app.rects.detail_inner = inner;
            f.render_widget(b, area);
            if let Some(err) = &app.d1.grid_error {
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(
                            one_line(&app.d1.last_sql, inner.width as usize),
                            dim(),
                        )),
                        Line::from(Span::styled(err.clone(), Style::new().fg(theme::error()))),
                    ])
                    .wrap(Wrap { trim: false }),
                    inner,
                );
                return;
            }
            let Some(g) = &app.d1.grid else {
                f.render_widget(Paragraph::new(Span::styled("running…", dim())), inner);
                return;
            };
            let [sqlline, grid] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
            let meta = format!(
                "{} rows · {:.1} ms · read {} written {}",
                g.rows.len(),
                g.meta.duration.unwrap_or(0.0),
                g.meta.rows_read.unwrap_or(0),
                g.meta.rows_written.unwrap_or(0)
            );
            let sqlw = (sqlline.width as usize).saturating_sub(meta.len() + 2);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        pad(&one_line(&g.sql, sqlw), sqlw),
                        Style::new().fg(theme::string()),
                    ),
                    Span::raw("  "),
                    Span::styled(meta, dim()),
                ])),
                sqlline,
            );
            if g.columns.is_empty() {
                f.render_widget(
                    Paragraph::new(Span::styled("statement ran, no result set", dim())),
                    grid,
                );
                return;
            }
            // header + rows with horizontal scroll by column index
            let w = grid.width as usize;
            let hs = app.d1.hscroll.min(g.columns.len().saturating_sub(1));
            let mut header = String::new();
            let mut col_slots: Vec<(usize, usize)> = Vec::new(); // (col idx, width)
            let mut used = 0;
            for (i, c) in g.columns.iter().enumerate().skip(hs) {
                let cw = g
                    .widths
                    .get(i)
                    .copied()
                    .unwrap_or(8)
                    .min(w.saturating_sub(used))
                    .max(1);
                if used + cw + 1 > w && !col_slots.is_empty() {
                    break;
                }
                header.push_str(&pad(c, cw));
                header.push(' ');
                col_slots.push((i, cw));
                used += cw + 1;
            }
            let body_h = (grid.height as usize).saturating_sub(1);
            let sel = app.d1.row.sel;
            let scroll = &mut app.d1.row.scroll;
            if sel < *scroll {
                *scroll = sel;
            } else if body_h > 0 && sel >= *scroll + body_h {
                *scroll = sel + 1 - body_h;
            }
            let mut lines = vec![Line::from(Span::styled(
                pad(&header, w),
                dim().add_modifier(Modifier::UNDERLINED),
            ))];
            for (ri, row) in g.rows.iter().enumerate().skip(*scroll).take(body_h) {
                let mut spans = Vec::new();
                for (ci, cw) in &col_slots {
                    let v = row.get(*ci).cloned().unwrap_or(serde_json::Value::Null);
                    let mut st = cell_style(&v);
                    if ri == sel {
                        st = st.bg(if focused {
                            theme::sel_bg()
                        } else {
                            theme::sel_bg_dim()
                        });
                    }
                    spans.push(Span::styled(pad(&one_line(&cell_text(&v), *cw), *cw), st));
                    spans.push(Span::styled(
                        " ",
                        if ri == sel {
                            sel_style(focused)
                        } else {
                            text()
                        },
                    ));
                }
                if ri == sel && used < w {
                    spans.push(Span::styled(" ".repeat(w - used), sel_style(focused)));
                }
                lines.push(Line::from(spans));
            }
            f.render_widget(Paragraph::new(lines), grid);
        }
    }
}

fn draw_kv_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let ns = app.kv.list.current().cloned();
    let title = match &ns {
        Some(n) => format!(
            "{} · keys ({}{})",
            n.title,
            app.kv.keys.len(),
            if app.kv.cursor_token.is_some() {
                "+"
            } else {
                ""
            }
        ),
        None => "keys".into(),
    };
    let title = if app.kv.loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let mut b = block(title, focused).title_bottom(Line::from(Span::styled(
        " Enter view · e edit · n new · d delete · p prefix · m metadata ",
        dim(),
    )));
    if !app.kv.prefix.is_empty() {
        b = b.title(Line::from(Span::styled(
            format!(" prefix: {} ", app.kv.prefix),
            accent(),
        )));
    }
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    if let Some(e) = &app.kv.error {
        f.render_widget(
            Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::error())))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let w = inner.width as usize;
    let rows = Rows {
        header: format!(
            "{} {:>16} {:>4}",
            pad("key", w.saturating_sub(22)),
            "expires",
            "meta"
        ),
        rows: app
            .kv
            .keys
            .iter()
            .map(|k| {
                (
                    format!(
                        "{} {:>16} {:>4}",
                        pad(&k.name, w.saturating_sub(22)),
                        k.expiration
                            .map(|e| flaretop_core::auth::civil_from_unix(e as i64)
                                .replace('T', " ")[..16]
                                .to_string())
                            .unwrap_or_default(),
                        if k.metadata.is_some() { "▪" } else { "" }
                    ),
                    text(),
                )
            })
            .collect(),
    };
    if rows.rows.is_empty() {
        let msg = if app.kv.loading {
            "loading…"
        } else if ns.is_none() {
            "select a namespace"
        } else {
            "no keys"
        };
        f.render_widget(Paragraph::new(Span::styled(msg, dim())), inner);
        return;
    }
    let sel = app.kv.key.sel;
    render_rows(f, inner, &rows, sel, &mut app.kv.key.scroll, focused);
}

fn draw_r2_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let bucket = app.r2.list.current().cloned();
    let title = match &bucket {
        Some(b) => format!("{}/{}", b.name, app.r2.prefix),
        None => "objects".into(),
    };
    let title = if app.r2.loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let mut b = block(title, focused).title_bottom(Line::from(Span::styled(
        " Enter open · Backspace up · s download · u upload · d delete · p public ",
        dim(),
    )));
    if let Some(info) = &app.r2.info {
        let mut parts = Vec::new();
        if let Some(st) = &info.stats {
            parts.push(format!(
                "{} objects · {}",
                st.object_count,
                human_bytes(st.payload_size)
            ));
        }
        if let Some(m) = &info.managed
            && m.enabled
        {
            parts.push(format!("public {}", m.domain.clone().unwrap_or_default()));
        }
        for d in &info.custom {
            parts.push(d.domain.clone());
        }
        if !parts.is_empty() {
            b = b.title(Line::from(Span::styled(
                format!(" {} ", parts.join(" · ")),
                dim(),
            )));
        }
    }
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    if let Some(e) = &app.r2.error {
        f.render_widget(
            Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::error())))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let w = inner.width as usize;
    let prefix_len = app.r2.prefix.len();
    let rows = Rows {
        header: format!(
            "{} {:>9} {:>16}",
            pad("name", w.saturating_sub(27)),
            "size",
            "modified"
        ),
        rows: app
            .r2
            .entries
            .iter()
            .map(|e| match e {
                R2Entry::Up => (pad("..", w), dim()),
                R2Entry::Folder(p) => (
                    format!(
                        "{} {:>9} {:>16}",
                        pad(
                            &format!(
                                "{}/",
                                p.get(prefix_len..).unwrap_or(p).trim_end_matches('/')
                            ),
                            w.saturating_sub(27)
                        ),
                        "",
                        ""
                    ),
                    Style::new().fg(theme::key()),
                ),
                R2Entry::Object(o) => (
                    format!(
                        "{} {:>9} {:>16}",
                        pad(
                            o.key.get(prefix_len..).unwrap_or(&o.key),
                            w.saturating_sub(27)
                        ),
                        human_bytes(o.size),
                        o.last_modified
                            .as_deref()
                            .map(short_time)
                            .unwrap_or_default()
                    ),
                    text(),
                ),
            })
            .collect(),
    };
    if rows.rows.is_empty() {
        let msg = if app.r2.loading {
            "loading…"
        } else if bucket.is_none() {
            "select a bucket"
        } else {
            "empty"
        };
        f.render_widget(Paragraph::new(Span::styled(msg, dim())), inner);
        return;
    }
    let sel = app.r2.entry.sel;
    render_rows(f, inner, &rows, sel, &mut app.r2.entry.scroll, focused);
}

fn draw_queue_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let q = app.queues.list.current().cloned();
    let title = q
        .as_ref()
        .map(|q| q.queue_name.clone())
        .unwrap_or_else(|| "queue".into());
    let b = block(title, focused).title_bottom(Line::from(Span::styled(
        " m push · l pull · a ack · p pause · P purge ",
        dim(),
    )));
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    let Some(q) = q else {
        f.render_widget(Paragraph::new(Span::styled("select a queue", dim())), inner);
        return;
    };
    let mut lines: Vec<Line> = vec![
        kv_line("id", q.queue_id.clone()),
        kv_line(
            "created",
            q.created_on.as_deref().map(short_time).unwrap_or_default(),
        ),
        kv_line(
            "settings",
            format!(
                "delay {}s · retention {}h{}",
                q.delivery_delay(),
                q.retention() / 3600,
                if q.paused() {
                    " · DELIVERY PAUSED"
                } else {
                    ""
                }
            ),
        ),
    ];
    if let Some(b) = app.queues.backlog {
        lines.push(kv_line("backlog", format!("{b} messages (at last pull)")));
    }
    lines.push(Line::raw(""));
    lines.push(section_title(&format!("producers ({})", q.producers.len())));
    for p in &q.producers {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} {}",
                p.kind.clone().unwrap_or_default(),
                p.script.clone().unwrap_or_default()
            ),
            text(),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(section_title(&format!("consumers ({})", q.consumers.len())));
    for c in &q.consumers {
        lines.push(Line::from(Span::styled(
            format!("  {}", consumer_line(c)),
            text(),
        )));
    }
    if !app.queues.pulled.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_title(&format!(
            "pulled messages ({}) — Enter view · a ack · A ack all",
            app.queues.pulled.len()
        )));
    }
    let head_h = lines.len().min(inner.height as usize);
    let [head, list] =
        Layout::vertical([Constraint::Length(head_h as u16), Constraint::Min(0)]).areas(inner);
    f.render_widget(
        Paragraph::new(lines).scroll((app.queues.scroll as u16, 0)),
        head,
    );
    if !app.queues.pulled.is_empty() && list.height > 0 {
        let w = list.width as usize;
        let rows = Rows {
            header: format!("{:<9} {:>3} {}", "time", "try", "body"),
            rows: app
                .queues
                .pulled
                .iter()
                .map(|m| {
                    let body = match &m.body {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (
                        format!(
                            "{:<9} {:>3} {}",
                            m.timestamp_ms.map(clock_from_ms).unwrap_or_default(),
                            m.attempts,
                            one_line(&body, w.saturating_sub(14))
                        ),
                        text(),
                    )
                })
                .collect(),
        };
        let sel = app.queues.msg.sel;
        render_rows(f, list, &rows, sel, &mut app.queues.msg.scroll, focused);
    }
}

fn consumer_line(c: &queues::Consumer) -> String {
    let s = c.settings.clone().unwrap_or_default();
    let get = |k: &str| {
        s.get(k)
            .and_then(|v| v.as_u64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into())
    };
    format!(
        "{} {}  batch {} · retries {} · wait {}ms{}",
        c.kind.clone().unwrap_or_default(),
        c.script.clone().unwrap_or_default(),
        get("batch_size"),
        get("max_retries"),
        get("max_wait_time_ms"),
        c.dead_letter_queue
            .as_ref()
            .map(|d| format!(" · dlq {d}"))
            .unwrap_or_default()
    )
}

fn draw_do_detail(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let ns = app.dos.list.current().cloned();
    let title = match &ns {
        Some(n) => format!(
            "{} · objects ({}{})",
            n.class.clone().unwrap_or_default(),
            app.dos.objects.len(),
            if app.dos.cursor_token.is_some() {
                "+"
            } else {
                ""
            }
        ),
        None => "objects".into(),
    };
    let title = if app.dos.loading {
        format!("{title} {}", app.spinner())
    } else {
        title
    };
    let b = block(title, focused).title_bottom(Line::from(Span::styled(
        " y copy id · G load more · inventory only: the API exposes no storage ",
        dim(),
    )));
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    let Some(n) = ns else {
        f.render_widget(
            Paragraph::new(Span::styled("select a namespace", dim())),
            inner,
        );
        return;
    };
    let [info, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
    f.render_widget(
        Paragraph::new(vec![
            kv_line("namespace", n.id.clone()),
            kv_line(
                "storage",
                if n.use_sqlite.unwrap_or(false) {
                    "sqlite"
                } else {
                    "key-value"
                },
            ),
        ]),
        info,
    );
    let w = list.width as usize;
    let rows = Rows {
        header: format!("{} {:>6}", pad("object id", w.saturating_sub(7)), "data"),
        rows: app
            .dos
            .objects
            .iter()
            .map(|o| {
                (
                    format!(
                        "{} {:>6}",
                        pad(&o.id, w.saturating_sub(7)),
                        if o.has_stored_data { "yes" } else { "" }
                    ),
                    text(),
                )
            })
            .collect(),
    };
    if rows.rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                if app.dos.loading {
                    "loading…"
                } else {
                    "no objects"
                },
                dim(),
            )),
            list,
        );
        return;
    }
    let sel = app.dos.object.sel;
    render_rows(f, list, &rows, sel, &mut app.dos.object.scroll, focused);
}

fn draw_tail(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(t) = &mut app.tail else { return };
    let vis = t.visible();
    let title = format!(
        "tail · {} · {} events{} · {}",
        t.script,
        t.events.len(),
        if t.paused {
            format!(" · PAUSED (+{} buffered)", t.pending.len())
        } else {
            String::new()
        },
        t.status
    );
    let mut b = block(title, true).title_bottom(Line::from(Span::styled(
        " Esc stop · space pause · j/k select · Enter raw · y copy · f filter · c clear ",
        dim(),
    )));
    if !t.filter.is_empty() {
        b = b.title(Line::from(Span::styled(
            format!(" filter: {} ", t.filter),
            accent(),
        )));
    }
    let inner = b.inner(area);
    app.rects.detail_inner = inner;
    f.render_widget(b, area);
    let w = inner.width as usize;
    let h = inner.height as usize;
    // Each event renders as 1 + logs + exceptions lines. Build from the end
    // when following, so the newest is always visible.
    let mut lines: Vec<Line> = Vec::new();
    let sel_idx = t.sel.and_then(|s| vis.get(s).copied());
    let render_event = |i: usize, selected: bool| -> Vec<Line<'static>> {
        let e = &t.events[i];
        let color = match e.outcome.as_str() {
            "ok" => theme::ok(),
            "canceled" => theme::warn(),
            _ => theme::error(),
        };
        let timing = match (e.cpu_ms, e.wall_ms) {
            (Some(c), Some(wl)) => format!(" {c}ms cpu · {wl}ms"),
            _ => String::new(),
        };
        let mut out = vec![Line::from(vec![
            Span::styled(format!("{} ", clock_from_ms(e.timestamp_ms)), dim()),
            Span::styled(
                format!("{:<9} ", fit(&e.outcome, 9)),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                fit(&e.summary, w.saturating_sub(22 + timing.len())),
                if selected { sel_style(true) } else { text() },
            ),
            Span::styled(timing, dim()),
        ])];
        for (level, msg) in &e.logs {
            let lc = match level.as_str() {
                "error" => theme::error(),
                "warn" => theme::warn(),
                "debug" => theme::dim(),
                _ => theme::text(),
            };
            out.push(Line::from(vec![
                Span::styled(format!("{:>14}  ", fit(level, 14)), dim()),
                Span::styled(fit(msg, w.saturating_sub(17)), Style::new().fg(lc)),
            ]));
        }
        for ex in &e.exceptions {
            out.push(Line::from(vec![
                Span::styled("     exception  ", dim()),
                Span::styled(
                    fit(ex, w.saturating_sub(17)),
                    Style::new().fg(theme::error()),
                ),
            ]));
        }
        out
    };
    match t.sel {
        None => {
            // follow: fill from the bottom
            let mut acc: Vec<Vec<Line>> = Vec::new();
            let mut total = 0;
            for &i in vis.iter().rev() {
                let ls = render_event(i, false);
                total += ls.len();
                acc.push(ls);
                if total >= h {
                    break;
                }
            }
            for ls in acc.into_iter().rev() {
                lines.extend(ls);
            }
            let skip = lines.len().saturating_sub(h);
            lines = lines.into_iter().skip(skip).collect();
        }
        Some(s) => {
            // selection: start a bit above the selected event
            let start = s.saturating_sub(h / 3);
            for &i in vis.iter().skip(start) {
                let ls = render_event(i, Some(i) == sel_idx);
                lines.extend(ls);
                if lines.len() >= h {
                    break;
                }
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "waiting for events… (requests to the worker show up here)",
            dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let keys: &str = if app.tail.is_some() {
        "Esc stop  space pause  Enter raw  f filter  ? help"
    } else if app.filter_mode {
        "type to filter · Enter keep · Esc clear"
    } else {
        match (app.focus, app.section) {
            (Focus::Sidebar, _) => "j/k section  Enter open  1-7 jump  ? help  q quit",
            (_, Section::Pages) => {
                "Enter deployment  l build log  R rollback  b retry  y copy url  D delete  / filter  ? help"
            }
            (_, Section::Workers) => {
                "Enter detail  t tail  c code  v versions  s secret  k cron  w workers.dev  D delete  / filter  : palette  ? help"
            }
            (_, Section::D1) => {
                "Enter open  s sql  e/i/d row  n new db  D delete db  b bookmark  / filter  ? help"
            }
            (_, Section::Kv) => {
                "Enter view  e edit  n new  d delete  D delete prefix  p prefix  N new ns  X delete ns  ? help"
            }
            (_, Section::R2) => {
                "Enter open  ⌫ up  s download  u upload  d delete  p public  N new bucket  X delete bucket  ? help"
            }
            (_, Section::Queues) => {
                "m push  l pull  a ack  p pause  P purge  n new  D delete  ? help"
            }
            (_, Section::DurableObjects) => "Enter/y copy id  G load more  ? help",
        }
    };
    let mut line = vec![Span::styled(format!(" {keys}"), dim())];
    if let Some(cmd) = &app.last_cf {
        let w = (area.width as usize).saturating_sub(keys.len() + 4);
        if w > 20 {
            line.push(Span::styled(
                format!("  {}", fit(cmd, w)),
                Style::new().fg(theme::string()),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(line)), area);
}

fn centered(area: Rect, pct_w: u16, pct_h: u16, min_w: u16, min_h: u16) -> Rect {
    let w = ((area.width as u32 * pct_w as u32) / 100)
        .max(min_w as u32)
        .min(area.width as u32) as u16;
    let h = ((area.height as u32 * pct_h as u32) / 100)
        .max(min_h as u32)
        .min(area.height as u32) as u16;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_modal(f: &mut Frame, app: &mut App, area: Rect) {
    match &mut app.modal {
        Modal::None => {}
        Modal::Help => {
            let r = centered(area, 80, 90, 60, 20);
            f.render_widget(Clear, r);
            let b =
                block("help", true).title_bottom(Line::from(Span::styled(" Esc close ", dim())));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let lines: Vec<Line> = HELP
                .lines()
                .map(|l| {
                    if l.ends_with(':') && !l.starts_with(' ') {
                        Line::from(Span::styled(
                            l.to_string(),
                            accent().add_modifier(Modifier::BOLD),
                        ))
                    } else {
                        Line::from(Span::styled(l.to_string(), text()))
                    }
                })
                .collect();
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Modal::OpsLog { scroll } => {
            let r = centered(area, 80, 70, 60, 10);
            f.render_widget(Clear, r);
            let b = block("operations log", true).title_bottom(Line::from(Span::styled(
                " j/k scroll · G end · Esc close ",
                dim(),
            )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let h = inner.height as usize;
            let max = app.ops_log.len().saturating_sub(h);
            if *scroll > max {
                *scroll = max;
            }
            let lines: Vec<Line> = app
                .ops_log
                .iter()
                .skip(*scroll)
                .take(h)
                .map(|l| {
                    Line::from(Span::styled(
                        l.clone(),
                        if l.contains(" ERROR ") {
                            Style::new().fg(theme::error())
                        } else {
                            text()
                        },
                    ))
                })
                .collect();
            let lines = if lines.is_empty() {
                vec![Line::from(Span::styled("no operations yet", dim()))]
            } else {
                lines
            };
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::Accounts { items, selected } => {
            let r = centered(area, 50, 40, 40, 6);
            f.render_widget(Clear, r);
            let b = block("accounts", true).title_bottom(Line::from(Span::styled(
                " Enter switch · Esc close ",
                dim(),
            )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let lines: Vec<Line> = items
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let st = if i == *selected {
                        sel_style(true)
                    } else {
                        text()
                    };
                    Line::from(Span::styled(
                        pad(&format!("{}  {}", a.name, a.id), inner.width as usize),
                        st,
                    ))
                })
                .collect();
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::Profiles { items, selected } => {
            let r = centered(area, 50, 40, 40, 6);
            f.render_widget(Clear, r);
            let b = block("profiles (saved tokens)", true).title_bottom(Line::from(Span::styled(
                " Enter switch · Esc close · add with `flaretop --login --profile NAME` ",
                dim(),
            )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let lines: Vec<Line> = if items.is_empty() {
                vec![Line::from(Span::styled(
                    "no saved profiles; run `flaretop --login`",
                    dim(),
                ))]
            } else {
                items
                    .iter()
                    .enumerate()
                    .map(|(i, (name, p))| {
                        let st = if i == *selected {
                            sel_style(true)
                        } else {
                            text()
                        };
                        Line::from(Span::styled(
                            pad(
                                &format!(
                                    "{name}  account {}",
                                    p.account_id.as_deref().unwrap_or("-")
                                ),
                                inner.width as usize,
                            ),
                            st,
                        ))
                    })
                    .collect()
            };
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::Confirm(c) => {
            let h = (c.body.len() + 5) as u16;
            let r = centered(area, 60, 10, 50, h);
            f.render_widget(Clear, r);
            let color = if c.danger {
                theme::error()
            } else {
                theme::accent()
            };
            let b = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::new().fg(color))
                .title(Line::from(Span::styled(
                    format!(" {} ", c.title),
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                )))
                .title_bottom(Line::from(Span::styled(
                    if c.must_type.is_some() {
                        " Enter confirm · Esc cancel "
                    } else {
                        " y/Enter confirm · n/Esc cancel "
                    },
                    dim(),
                )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let mut lines: Vec<Line> = c
                .body
                .iter()
                .map(|l| Line::from(Span::styled(l.clone(), text())))
                .collect();
            if c.must_type.is_some() {
                lines.push(Line::raw(""));
                let mut spans = vec![Span::styled("> ", accent())];
                spans.extend(c.input.spans(true, inner.width.saturating_sub(2)));
                lines.push(Line::from(spans));
            }
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Modal::Prompt(p) => {
            let r = centered(area, 60, 10, 50, 6);
            f.render_widget(Clear, r);
            let b = block(p.title.clone(), true)
                .title_bottom(Line::from(Span::styled(" Enter ok · Esc cancel ", dim())));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let mut lines = vec![Line::from(Span::styled(p.label.clone(), dim()))];
            let mut spans = vec![Span::styled("> ", accent())];
            if p.secret {
                spans.push(Span::styled(
                    "•".repeat(p.input.text.chars().count()),
                    text(),
                ));
            } else {
                spans.extend(p.input.spans(true, inner.width.saturating_sub(2)));
            }
            lines.push(Line::from(spans));
            if let Some(e) = &p.error {
                lines.push(Line::from(Span::styled(
                    e.clone(),
                    Style::new().fg(theme::error()),
                )));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::Editor(ed) => {
            let r = centered(area, 85, 80, 60, 12);
            f.render_widget(Clear, r);
            let b = block(ed.title.clone(), true)
                .title_bottom(Line::from(Span::styled(format!(" {} ", ed.hint), dim())));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let [body, err] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(if ed.error.is_some() { 1 } else { 0 }),
            ])
            .areas(inner);
            ed.area.render(f, body, true);
            if let Some(e) = &ed.error {
                f.render_widget(
                    Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::error()))),
                    err,
                );
            }
        }
        Modal::Viewer(v) => {
            let r = centered(area, 90, 90, 60, 12);
            f.render_widget(Clear, r);
            let b = block(v.title.clone(), true).title_bottom(Line::from(Span::styled(
                format!(
                    " {}/{} · j/k scroll · h/l pan · y copy · Esc close ",
                    v.scroll + 1,
                    v.lines.len().max(1)
                ),
                dim(),
            )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let h = inner.height as usize;
            let lines: Vec<Line> = v.lines.iter().skip(v.scroll).take(h).cloned().collect();
            f.render_widget(Paragraph::new(lines).scroll((0, v.hscroll as u16)), inner);
        }
        Modal::Palette(p) => {
            let r = centered(area, 60, 60, 50, 10);
            f.render_widget(Clear, r);
            let b = block(p.title.clone(), true).title_bottom(Line::from(Span::styled(
                " type to search · Enter run · Esc close ",
                dim(),
            )));
            let inner = b.inner(r);
            f.render_widget(b, r);
            let mut lines = vec![Line::from({
                let mut s = vec![Span::styled("> ", accent())];
                s.extend(p.input.spans(true, inner.width.saturating_sub(2)));
                s
            })];
            let h = (inner.height as usize).saturating_sub(1);
            let start = p.selected.saturating_sub(h.saturating_sub(1));
            for (i, &ai) in p.filtered.iter().enumerate().skip(start).take(h) {
                let st = if i == p.selected {
                    sel_style(true)
                } else {
                    text()
                };
                lines.push(Line::from(Span::styled(
                    pad(&p.actions[ai].0, inner.width as usize),
                    st,
                )));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

fn draw_toasts(f: &mut Frame, app: &App, area: Rect) {
    let Some(t) = app.toasts.last() else { return };
    let w = (t.text.chars().count() as u16 + 4)
        .min(area.width.saturating_sub(2))
        .max(10);
    let r = Rect {
        x: area.x + area.width.saturating_sub(w + 1),
        y: area.y + area.height.saturating_sub(4),
        width: w,
        height: 3,
    };
    f.render_widget(Clear, r);
    let color = if t.error { theme::error() } else { theme::ok() };
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color));
    let inner = b.inner(r);
    f.render_widget(b, r);
    f.render_widget(
        Paragraph::new(Span::styled(
            fit(&t.text, inner.width as usize),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        )),
        inner,
    );
}

const HELP: &str = "\
global:
  1-6 / Tab / h,l   jump to a section · cycle panes · move between panes
  j k g G PgUp PgDn move · / filter the list · r refresh · : or Ctrl-P palette
  C copy the equivalent cf command · L operations log · A switch account
  P switch profile (saved token) · q quit
  [ ] narrower / wider list · { } sidebar · = reset · or drag a divider with the mouse

workers:
  Enter detail · v versions (Enter deploy 100% · % gradual) · t live tail
  c view deployed code · s set secret · x delete secret · k cron triggers
  w toggle workers.dev · y copy workers.dev url · D delete worker

pages:
  Enter deployment details · l build log · R roll production back to it
  b retry build · y copy url · D delete project

d1:
  Enter open table (200 rows) · s SQL console (Ctrl-S runs; writes confirm)
  in a result: e edit row · i insert · d delete · Enter row as JSON · h/l columns
  n new database · D delete database · b time-travel bookmark

kv:
  Enter view · e edit value · m edit metadata · n new key · d delete key
  p prefix filter · G load next page · D delete all loaded keys
  N new namespace · X delete namespace

r2:
  Enter open folder / view object · Backspace up · s download · u upload
  d delete object · p toggle r2.dev public access · N new bucket · X delete bucket
  (objects over 300 MB need the S3 API)

queues:
  m push message · l pull (needs an http_pull consumer; leases 30 s) · a/A ack
  p pause or resume delivery · P purge · n new queue · D delete queue

durable objects:
  inventory only: namespaces, object ids, has-data flag. y copies an id.

tail:
  space pause · j/k select (G follows again) · Enter raw event · y copy · f filter · c clear

every destructive action asks for confirmation; dangerous ones require typing a name.
--readonly (or the palette) blocks every write at the I/O layer.
";
