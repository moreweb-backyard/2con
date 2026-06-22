use reqwest::{Client, Proxy};
use std::time::Duration;

pub async fn test_proxy_connection() -> bool {
    let proxy = match Proxy::all("socks5h://127.0.0.1:10808") {
        Ok(p) => p,
        Err(_) => return false,
    };

    let client = match Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(5))
        .build() {
        Ok(c) => c,
        Err(_) => return false,
    };

    match client.get("https://1.1.1.1").send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
