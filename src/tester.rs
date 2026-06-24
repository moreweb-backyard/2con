use reqwest::{Client, Proxy};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub fn tcp_ping(address: &str, port: u16) -> Option<u128> {
    let target = format!("{}:{}", address, port);
    let addr: SocketAddr = target.parse().ok().or_else(|| {
        // Fallback for simple domain resolution
        std::net::ToSocketAddrs::to_socket_addrs(&target)
            .ok()?
            .next()
    })?;

    let start = Instant::now();
    match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(_) => Some(start.elapsed().as_millis()),
        Err(_) => None,
    }
}

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
