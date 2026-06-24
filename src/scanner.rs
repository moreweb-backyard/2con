use reqwest::Client;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub ip: String,
    pub latency: Duration,
    pub is_valid: bool,
    pub colo: String,
}

pub async fn test_ip(ip: &str, port: u16, timeout_ms: u64) -> ScanResult {
    let mut result = ScanResult {
        ip: ip.to_string(),
        latency: Duration::from_secs(999),
        is_valid: false,
        colo: String::new(),
    };

    let start = Instant::now();
    let addr = format!("{}:{}", ip, port);

    // Step 1: TCP Connect
    let tcp_conn = timeout(Duration::from_millis(timeout_ms), TcpStream::connect(&addr)).await;
    match tcp_conn {
        Ok(Ok(_)) => {
            // TCP successful, move to HTTP test
        }
        _ => return result, // TCP failed or timeout
    }

    // Step 2: HTTP GET /cdn-cgi/trace using the IP directly but with SNI
    // Since reqwest handles TLS, we can construct a client that resolves the host to this IP
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .resolve("speed.cloudflare.com", addr.parse().unwrap())
        .timeout(Duration::from_millis(timeout_ms))
        .build();

    if let Ok(client) = client {
        let scheme = if port == 80 { "http" } else { "https" };
        let url = format!("{}://speed.cloudflare.com/cdn-cgi/trace", scheme);

        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    for line in text.lines() {
                        if line.starts_with("colo=") {
                            result.colo = line.trim_start_matches("colo=").to_string();
                            result.is_valid = true;
                            result.latency = start.elapsed();
                            break;
                        }
                    }
                }
            }
        }
    }

    result
}

// Generate a basic list of CF IPs
pub fn get_common_cf_ips() -> Vec<String> {
    // For now, hardcode a few known ranges or IPs to scan for demonstration
    // In a real app, this would generate thousands of IPs from CF CIDRs
    vec![
        "1.1.1.1".into(),
        "1.0.0.1".into(),
        "104.16.132.229".into(),
        "104.16.133.229".into(),
        "104.17.132.229".into(),
    ]
}
