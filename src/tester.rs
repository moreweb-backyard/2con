use reqwest::{Client, Proxy};
use std::time::{Duration, Instant};

pub async fn real_delay(socks_port: u16) -> Option<u128> {
    let proxy_url = format!("socks5h://127.0.0.1:{}", socks_port);
    let proxy = match Proxy::all(&proxy_url) {
        Ok(p) => p,
        Err(_) => return None,
    };

    let client = match Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return None,
    };

    let start = Instant::now();
    match client
        .get("https://www.gstatic.com/generate_204")
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                Some(start.elapsed().as_millis())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}
