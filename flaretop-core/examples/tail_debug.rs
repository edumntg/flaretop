//! Prints every raw frame from a Worker tail for 25 seconds.
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    flaretop_core::init_crypto();
    let token = std::env::var("CLOUDFLARE_API_TOKEN").unwrap();
    let account = std::env::var("CLOUDFLARE_ACCOUNT_ID").unwrap();
    let script = std::env::args().nth(1).unwrap_or("vecflare".into());
    let c = flaretop_core::api::Client::new(&token).unwrap();
    let t = flaretop_core::tail::create(&c, &account, &script, &Default::default())
        .await
        .unwrap();
    println!("tail {} url {} expires {:?}", t.id, t.url, t.expires_at);
    let mut req = t.url.as_str().into_client_request().unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "trace-v1".parse().unwrap());
    req.headers_mut()
        .insert("User-Agent", "flaretop-debug".parse().unwrap());
    let (ws, resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    println!("ws status {} headers {:?}", resp.status(), resp.headers());
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::Text(r#"{"debug":false}"#.into()))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            f = stream.next() => match f {
                Some(Ok(Message::Text(t))) => println!("TEXT {}", &t[..t.len().min(300)]),
                Some(Ok(Message::Binary(b))) => println!("BIN {} bytes: {}", b.len(), String::from_utf8_lossy(&b[..b.len().min(300)])),
                Some(Ok(m)) => println!("OTHER {m:?}"),
                Some(Err(e)) => { println!("ERR {e}"); break }
                None => { println!("closed"); break }
            }
        }
    }
    let _ = flaretop_core::tail::delete(&c, &account, &script, &t.id).await;
}
