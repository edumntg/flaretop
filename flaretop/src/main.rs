mod actions;
mod app;
mod cfcmd;
mod config;
mod event;
mod input;
mod json_view;
mod keys;
mod modal;
mod splash;
mod term;
mod textarea;
mod theme;
mod ui;
mod util;

use anyhow::{Context, Result};
use crossterm::event::Event;
use flaretop_core::api::Client;
use flaretop_core::auth::{self, Account, CredSource, Credential};

const USAGE: &str = "\
flaretop — a fast, lightweight terminal UI for Cloudflare

USAGE:
    flaretop [OPTIONS]

OPTIONS:
    -a, --account ID    account id to open (default: last used, or a picker)
    -p, --profile NAME  saved token to use (see --login); default: last used
    -r, --readonly      block all write operations
        --token TOKEN   use this API token (prefer CLOUDFLARE_API_TOKEN)
        --cli-login     also accept tokens stored by `cf auth login` / wrangler
        --theme NAME    cloudflare (default) | cloudflare-light | dark | light |
                        claude-dark | claude-light | termius | high-contrast
        --no-anim       skip the start-up animation (also FLARETOP_NO_ANIM=1)
        --login         paste and save an API token as a profile, then exit
                        (combine with --profile NAME and --account ID)
        --logout        forget a saved profile (default: the one named default), then exit
        --profiles      list saved profiles, then exit
    -h, --help          print this help
    -V, --version       print version

KEYS:
    press ? inside the app for the full keymap
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    flaretop_core::init_crypto();
    let mut account_arg: Option<String> = None;
    let mut read_only = false;
    let mut cli_theme: Option<String> = None;
    let mut cli_token: Option<String> = None;
    let mut cli_login = false;
    let mut do_login = false;
    let mut do_logout = false;
    let mut do_profiles = false;
    let mut no_anim = false;
    let mut profile_arg: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("flaretop {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-r" | "--readonly" | "--read-only" => read_only = true,
            "-a" | "--account" => account_arg = args.next(),
            "--theme" => cli_theme = args.next(),
            "--token" => cli_token = args.next(),
            "--cli-login" => cli_login = true,
            "--login" => do_login = true,
            "--logout" => do_logout = true,
            "--profiles" => do_profiles = true,
            "--no-anim" => no_anim = true,
            "-p" | "--profile" => profile_arg = args.next(),
            other => {
                eprintln!("unknown option: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let mut config = config::load();
    if do_profiles {
        let profiles = auth::profiles();
        if profiles.is_empty() {
            println!("no saved profiles (run `flaretop --login`)");
        }
        for (name, p) in profiles {
            println!(
                "{name}{}  account: {}",
                if config.profile.as_deref() == Some(&name) {
                    " (last used)"
                } else {
                    ""
                },
                p.account_id.as_deref().unwrap_or("-")
            );
        }
        return Ok(());
    }
    if do_logout {
        let name = profile_arg.unwrap_or_else(|| auth::DEFAULT_PROFILE.to_string());
        if auth::forget_profile(&name)? {
            println!("profile {name} removed");
        } else {
            println!("no profile named {name}");
        }
        return Ok(());
    }
    if do_login {
        let name = match profile_arg {
            Some(n) => n,
            None => {
                eprint!("Profile name [{}]: ", auth::DEFAULT_PROFILE);
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                let n = line.trim().to_string();
                if n.is_empty() {
                    auth::DEFAULT_PROFILE.to_string()
                } else {
                    n
                }
            }
        };
        let token = read_token_stdin()?;
        let client = Client::new(&token)?;
        let status = auth::verify(&client)
            .await
            .context("token verification failed")?;
        let account_id = match &account_arg {
            Some(a) => Some(a.clone()),
            None => {
                let accounts = auth::accounts(&client).await.unwrap_or_default();
                match accounts.len() {
                    0 => None,
                    1 => Some(accounts[0].id.clone()),
                    _ => Some(pick_account(&accounts)?.id),
                }
            }
        };
        let path = auth::save_profile(&name, &token, account_id.as_deref())?;
        println!(
            "token {} saved as profile \"{name}\" in {}",
            status.status,
            path.display()
        );
        return Ok(());
    }
    if let Some(name) = cli_theme.as_deref().or(config.theme.as_deref())
        && !theme::set_by_name(name)
    {
        eprintln!(
            "unknown theme \"{name}\" — available: {}",
            theme::NAMES.join(", ")
        );
        std::process::exit(2);
    }

    // Credentials: explicit --token, then discovery in precedence order.
    let mut cred: Option<Credential> = cli_token.map(|t| Credential {
        token: t,
        source: CredSource::Prompt,
        expires: None,
        path: None,
    });
    let discovered = auth::discover();
    let allow_cli = cli_login || config.use_cli_logins;
    let profiles = auth::profiles();
    let mut profile_name: Option<String> = None;
    let mut profile_account: Option<String> = None;
    if cred.is_none()
        && let Some(name) = profile_arg.clone()
    {
        match auth::load_profile(&name) {
            Some(c) => {
                profile_account = profiles
                    .iter()
                    .find(|(n, _)| *n == name)
                    .and_then(|(_, p)| p.account_id.clone());
                profile_name = Some(name);
                cred = Some(c);
            }
            None => {
                eprintln!(
                    "no profile named {name}; saved profiles: {}",
                    profiles
                        .iter()
                        .map(|(n, _)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(2);
            }
        }
    }
    if cred.is_none() {
        cred = discovered
            .iter()
            .find(|c| c.source == CredSource::Env)
            .cloned();
    }
    if cred.is_none() && !profiles.is_empty() {
        let pick = config
            .profile
            .clone()
            .filter(|p| profiles.iter().any(|(n, _)| n == p))
            .or_else(|| (profiles.len() == 1).then(|| profiles[0].0.clone()))
            .or_else(|| {
                profiles
                    .iter()
                    .find(|(n, _)| n == auth::DEFAULT_PROFILE)
                    .map(|(n, _)| n.clone())
            });
        let name = match pick {
            Some(n) => n,
            None => {
                eprintln!("Saved profiles:");
                for (i, (n, p)) in profiles.iter().enumerate() {
                    eprintln!(
                        "  {}. {n}  (account {})",
                        i + 1,
                        p.account_id.as_deref().unwrap_or("-")
                    );
                }
                eprint!("Choose [1-{}]: ", profiles.len());
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                let n: usize = line.trim().parse().unwrap_or(1);
                profiles
                    .get(n.saturating_sub(1))
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| profiles[0].0.clone())
            }
        };
        if let Some(c) = auth::load_profile(&name) {
            profile_account = profiles
                .iter()
                .find(|(n, _)| *n == name)
                .and_then(|(_, p)| p.account_id.clone());
            profile_name = Some(name);
            cred = Some(c);
        }
    }
    if cred.is_none() {
        cred = discovered
            .iter()
            .find(|c| {
                matches!(c.source, CredSource::CfCli | CredSource::Wrangler)
                    && allow_cli
                    && !c.is_expired()
            })
            .cloned();
    }
    let cred = match cred {
        Some(c) => c,
        None => {
            // Offer the CLI logins we found, or ask for a token.
            let cli: Vec<&Credential> = discovered
                .iter()
                .filter(|c| matches!(c.source, CredSource::CfCli | CredSource::Wrangler))
                .collect();
            if let Some(c) = cli.iter().find(|c| !c.is_expired()) {
                eprintln!(
                    "No API token found. A {} token exists at {}.",
                    c.source.label(),
                    c.path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                eprint!("Use it for this session? [Y/n] ");
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                if !line.trim().to_lowercase().starts_with('n') {
                    (*c).clone()
                } else {
                    prompt_token().await?
                }
            } else {
                if let Some(c) = cli.first() {
                    eprintln!(
                        "A {} login exists but its token expired; run `{}` to refresh it, or paste an API token.",
                        c.source.label(),
                        if c.source == CredSource::CfCli {
                            "cf auth login"
                        } else {
                            "wrangler login"
                        }
                    );
                }
                prompt_token().await?
            }
        }
    };

    let animate = splash::enabled(no_anim);
    if animate {
        let _ = splash::intro("verifying token…");
    }
    let (cred, client, status) = verify_or_refresh(cred).await;
    if animate {
        splash::status("listing accounts…");
    }
    if status.status != "active" {
        eprintln!("token status is {}", status.status);
        std::process::exit(1);
    }

    let accounts = match auth::accounts(&client).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("could not list accounts: {e}");
            Vec::new()
        }
    };
    let wanted = account_arg
        .or_else(|| profile_account.clone())
        .or_else(|| std::env::var("CLOUDFLARE_ACCOUNT_ID").ok())
        .or_else(|| config.account_id.clone());
    let account = match (&wanted, accounts.len()) {
        (Some(id), _) => accounts
            .iter()
            .find(|a| &a.id == id)
            .cloned()
            .unwrap_or_else(|| Account {
                id: id.clone(),
                name: "account".into(),
            }),
        (None, 1) => accounts[0].clone(),
        (None, 0) => {
            eprintln!(
                "this token cannot list accounts; pass --account ID or set CLOUDFLARE_ACCOUNT_ID"
            );
            std::process::exit(1);
        }
        (None, _) => pick_account(&accounts)?,
    };
    config.account_id = Some(account.id.clone());
    if profile_name.is_some() {
        config.profile = profile_name.clone();
    }
    let _ = config::save(&config);

    let handle = flaretop_core::actor::spawn(client.clone(), account.id.clone(), read_only);
    let mut app = app::App::new(handle, client, account, accounts, &cred, read_only, config);
    app.profile = profile_name;

    term::install_panic_hook();
    let mut terminal = term::init()?;
    let result = run(&mut terminal, &mut app).await;
    term::restore();
    result
}

async fn run(terminal: &mut term::Term, app: &mut app::App) -> Result<()> {
    let mut input = event::input_channel();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    app.ensure_loaded();
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        tokio::select! {
            ev = input.recv() => {
                match ev {
                    Some(Event::Key(k)) => keys::on_key(app, k),
                    Some(Event::Mouse(m)) => keys::on_mouse(app, m),
                    Some(Event::Resize(_, _)) => {}
                    Some(_) => {}
                    None => break,
                }
            }
            ev = app.handle.rx.recv() => {
                match ev {
                    Some(ev) => app.on_core(ev),
                    None => break,
                }
            }
            _ = tick.tick() => app.on_tick(),
        }
        // Drain whatever else is ready so one frame covers a burst.
        while let Ok(ev) = app.handle.rx.try_recv() {
            app.on_core(ev);
        }
        if app.should_quit {
            break;
        }
    }
    // Stop a live tail cleanly (deletes the server-side tail).
    if app.tail.is_some() {
        let _ = app
            .handle
            .tx
            .send((0, flaretop_core::types::Command::StopTail))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(())
}

/// Verify the token; when a `cf`/wrangler login is rejected, ask that CLI
/// to refresh it (they only refresh when they run a command), re-read the
/// file and try once more.
async fn verify_or_refresh(mut cred: Credential) -> (Credential, Client, auth::TokenStatus) {
    for attempt in 0..2 {
        let client = match Client::new(&cred.token) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        match auth::verify(&client).await {
            Ok(s) => return (cred, client, s),
            Err(e) => {
                let cli = match cred.source {
                    CredSource::CfCli => Some(("cf", "cf auth login --force")),
                    CredSource::Wrangler => Some(("wrangler", "wrangler login")),
                    _ => None,
                };
                match (cli, attempt) {
                    (Some((bin, relogin)), 0) => {
                        eprintln!(
                            "the {} token was rejected ({e}); asking `{bin}` to refresh it…",
                            cred.source.label()
                        );
                        let refreshed = std::process::Command::new(bin)
                            .args(["auth", "whoami"])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .map(|st| st.success())
                            .unwrap_or(false);
                        let reloaded = if cred.source == CredSource::CfCli {
                            auth::load_cf_cli()
                        } else {
                            auth::load_wrangler()
                        };
                        match (refreshed, reloaded) {
                            (true, Some(c)) if c.token != cred.token => cred = c,
                            _ => {
                                eprintln!(
                                    "could not refresh it. Run `{relogin}` and start flaretop again, or use an API token (see README)."
                                );
                                std::process::exit(1);
                            }
                        }
                    }
                    _ => {
                        eprintln!("token from {} is not valid: {e}", cred.source.label());
                        if cred.source == CredSource::ConfigFile {
                            eprintln!("run `flaretop --logout` to forget it");
                        }
                        if let Some((_, relogin)) = cli {
                            eprintln!("run `{relogin}`, or use an API token (see README)");
                        }
                        std::process::exit(1);
                    }
                }
            }
        }
    }
    unreachable!()
}

fn read_token_stdin() -> Result<String> {
    eprint!("Paste an API token (input hidden): ");
    let token = read_hidden()?;
    if token.trim().is_empty() {
        anyhow::bail!("no token given");
    }
    Ok(token.trim().to_string())
}

/// Read a line without echo when stdin is a terminal; a plain line when piped.
fn read_hidden() -> Result<String> {
    use std::io::BufRead as _;
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return Ok(line);
    }
    let _ = crossterm::terminal::enable_raw_mode();
    let mut out = String::new();
    loop {
        match crossterm::event::read()? {
            Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press => match k.code {
                crossterm::event::KeyCode::Enter => break,
                crossterm::event::KeyCode::Char('c')
                    if k.modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    let _ = crossterm::terminal::disable_raw_mode();
                    anyhow::bail!("cancelled");
                }
                crossterm::event::KeyCode::Char(c) => out.push(c),
                crossterm::event::KeyCode::Backspace => {
                    out.pop();
                }
                _ => {}
            },
            Event::Paste(s) => out.push_str(&s),
            _ => {}
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
    eprintln!();
    Ok(out)
}

async fn prompt_token() -> Result<Credential> {
    eprintln!(
        "Create one at https://dash.cloudflare.com/profile/api-tokens (template: Edit Cloudflare Workers, plus D1/KV/R2/Queues as needed)."
    );
    let token = read_token_stdin()?;
    let client = Client::new(&token)?;
    auth::verify(&client)
        .await
        .context("token verification failed")?;
    eprint!(
        "Save it to {}? [Y/n] ",
        auth::credentials_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let path = if !line.trim().to_lowercase().starts_with('n') {
        Some(auth::save_token(&token)?)
    } else {
        None
    };
    Ok(Credential {
        token,
        source: if path.is_some() {
            CredSource::ConfigFile
        } else {
            CredSource::Prompt
        },
        expires: None,
        path,
    })
}

fn pick_account(accounts: &[Account]) -> Result<Account> {
    eprintln!("Accounts:");
    for (i, a) in accounts.iter().enumerate() {
        eprintln!("  {}. {} ({})", i + 1, a.name, a.id);
    }
    eprint!("Choose [1-{}]: ", accounts.len());
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let n: usize = line.trim().parse().unwrap_or(1);
    Ok(accounts
        .get(n.saturating_sub(1))
        .cloned()
        .unwrap_or_else(|| accounts[0].clone()))
}
