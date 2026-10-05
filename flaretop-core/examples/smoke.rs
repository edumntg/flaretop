//! Read-only smoke test against a real account. Never prints the token.
//! Usage: CLOUDFLARE_API_TOKEN=... CLOUDFLARE_ACCOUNT_ID=... cargo run -p flaretop-core --example smoke

use flaretop_core::products::{d1, durable_objects as dos, kv, queues, r2, workers};
use flaretop_core::{api::Client, auth};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    flaretop_core::init_crypto();
    let token = std::env::var("CLOUDFLARE_API_TOKEN").expect("CLOUDFLARE_API_TOKEN");
    let account = std::env::var("CLOUDFLARE_ACCOUNT_ID").expect("CLOUDFLARE_ACCOUNT_ID");
    let c = Client::new(&token).unwrap();
    let t = std::time::Instant::now();
    println!("verify: {:?}", auth::verify(&c).await.map(|s| s.status));
    println!("accounts: {:?}", auth::accounts(&c).await.map(|a| a.len()));

    let scripts = workers::list(&c, &account).await.unwrap();
    println!(
        "workers: {} e.g. {:?}",
        scripts.len(),
        scripts.iter().take(3).map(|s| &s.id).collect::<Vec<_>>()
    );
    if let Some(s) = scripts.first() {
        let settings = workers::settings(&c, &account, &s.id).await.unwrap();
        println!(
            "  settings bindings: {}",
            flaretop_core::actor::bindings_rows(&settings).len()
        );
        println!(
            "  schedules: {:?}",
            workers::schedules(&c, &account, &s.id)
                .await
                .map(|v| v.len())
        );
        println!(
            "  secrets: {:?}",
            workers::secrets(&c, &account, &s.id).await.map(|v| v.len())
        );
        println!(
            "  subdomain: {:?}",
            workers::subdomain(&c, &account, &s.id)
                .await
                .map(|v| v.enabled)
        );
        let vs = workers::versions(&c, &account, &s.id).await.unwrap();
        println!(
            "  versions: {} first created {:?} by {:?}",
            vs.len(),
            vs.first().and_then(|v| v.created_on()),
            vs.first().and_then(|v| v.triggered_by())
        );
        let ds = workers::deployments(&c, &account, &s.id).await.unwrap();
        println!(
            "  deployments: {} first {:?}",
            ds.len(),
            ds.first()
                .map(|d| d.versions.iter().map(|v| v.percentage).collect::<Vec<_>>())
        );
        println!(
            "  stats24h: {:?}",
            workers::invocation_stats_24h(&c, &account, &s.id).await
        );
        let (body, ct) = workers::content(&c, &account, &s.id).await.unwrap();
        let parts = workers::split_multipart(&body, &ct);
        println!(
            "  content: {} parts, first {:?} ({} bytes)",
            parts.len(),
            parts.first().map(|p| &p.0),
            parts.first().map(|p| p.1.len()).unwrap_or(0)
        );
    }
    println!(
        "domains: {:?}",
        workers::domains(&c, &account).await.map(|v| v.len())
    );
    println!(
        "account subdomain: {:?}",
        workers::account_subdomain(&c, &account).await
    );

    let dbs = d1::list(&c, &account).await.unwrap();
    println!(
        "d1: {} e.g. {:?}",
        dbs.len(),
        dbs.iter().take(3).map(|d| &d.name).collect::<Vec<_>>()
    );
    if let Some(db) = dbs
        .iter()
        .find(|d| d.num_tables.unwrap_or(0) > 0)
        .or(dbs.first())
    {
        let tables = d1::tables(&c, &account, &db.uuid).await.unwrap();
        println!(
            "  {}: {} tables {:?}",
            db.name,
            tables.len(),
            tables.iter().take(5).map(|t| &t.name).collect::<Vec<_>>()
        );
        if let Some(tb) = tables.first() {
            let cols = d1::columns(&c, &account, &db.uuid, &tb.name).await.unwrap();
            println!(
                "  columns of {}: {:?}",
                tb.name,
                cols.iter()
                    .map(|c| format!("{}:{}{}", c.name, c.kind, if c.pk { "*" } else { "" }))
                    .collect::<Vec<_>>()
            );
            println!(
                "  count: {:?}",
                d1::count(&c, &account, &db.uuid, &tb.name).await
            );
            let rows = d1::raw(
                &c,
                &account,
                &db.uuid,
                &format!("SELECT * FROM {} LIMIT 2;", d1::quote_ident(&tb.name)),
                &[],
            )
            .await
            .unwrap();
            println!(
                "  raw: {} cols, {} rows, {:?} ms",
                rows[0].results.columns.len(),
                rows[0].results.rows.len(),
                rows[0].meta.duration
            );
        }
        println!(
            "  bookmark: {:?}",
            d1::bookmark(&c, &account, &db.uuid, None)
                .await
                .map(|b| b.len())
        );
    }

    let nss = kv::namespaces(&c, &account).await.unwrap();
    println!("kv: {} namespaces", nss.len());
    if let Some(ns) = nss.first() {
        let page = kv::keys(&c, &account, &ns.id, "", None, 10).await.unwrap();
        println!(
            "  {}: {} keys (cursor {:?})",
            ns.title,
            page.keys.len(),
            page.cursor.as_ref().map(|c| c.len())
        );
        if let Some(k) = page.keys.first() {
            let (v, exp) = kv::get_value(&c, &account, &ns.id, &k.name).await.unwrap();
            println!(
                "  value of first key: {} bytes, exp {:?}, metadata {:?}",
                v.len(),
                exp,
                kv::metadata(&c, &account, &ns.id, &k.name)
                    .await
                    .map(|m| m.is_null())
            );
        }
        println!(
            "  missing key -> {:?}",
            kv::get_value(&c, &account, &ns.id, "flaretop-does-not-exist")
                .await
                .err()
                .map(|e| e.is_not_found())
        );
    }

    let buckets = r2::buckets(&c, &account).await.unwrap();
    println!("r2: {} buckets", buckets.len());
    if let Some(b) = buckets.first() {
        let page = r2::objects(&c, &account, &b.name, "", Some("/"), None, 5)
            .await
            .unwrap();
        println!(
            "  {}: {} objects, {} folders, truncated {}",
            b.name,
            page.objects.len(),
            page.folders.len(),
            page.truncated
        );
        println!(
            "  managed: {:?}",
            r2::managed_domain(&c, &account, &b.name)
                .await
                .map(|m| m.enabled)
        );
        println!(
            "  stats: {:?}",
            r2::bucket_stats(&c, &account, &b.name).await
        );
        if let Some(o) = page.objects.first() {
            let (bytes, ct) = r2::get_object(&c, &account, &b.name, &o.key).await.unwrap();
            println!("  get {}: {} bytes {:?}", o.key.len(), bytes.len(), ct);
        }
    }

    let qs = queues::list(&c, &account).await.unwrap();
    println!(
        "queues: {} e.g. {:?}",
        qs.len(),
        qs.iter()
            .take(3)
            .map(|q| (&q.queue_name, q.consumers.len(), q.paused()))
            .collect::<Vec<_>>()
    );
    if let Some(q) = qs.first() {
        println!(
            "  backlog: {:?}",
            queues::backlog(&c, &account, &q.queue_id).await
        );
    }

    let dns = dos::namespaces(&c, &account).await.unwrap();
    println!("do: {} namespaces", dns.len());
    if let Some(n) = dns.first() {
        println!(
            "  objects: {:?}",
            dos::objects(&c, &account, &n.id, None)
                .await
                .map(|p| (p.objects.len(), p.cursor.is_some()))
        );
    }
    println!("requests used: {} in {:?}", c.budget_used(), t.elapsed());
}
