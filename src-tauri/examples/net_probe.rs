//! 网络探针：用**和应用完全一样**的 HTTP 配置去连一次 GitHub。
//!
//! 存在理由：更新器故意"失败不吭声"（网络不通不是用户的错），于是它出问题时
//! 现场什么都不剩。这个例子把同一套 reqwest 配置搬出来单独跑，错在哪一目了然 ——
//! 尤其是 TLS：`rustls-tls` 只认内置的 Mozilla 根证书，而本机这种带 TLS 拦截的
//! 环境（杀软、公司代理、沙箱）用的是自签 CA，只有走系统证书库才握得上手。
//!
//! 跑法：
//!     cd src-tauri && cargo run --example net_probe

use std::time::Duration;

const API: &str = "https://api.github.com/repos/WW-Ares/WSight/releases/latest";

#[tokio::main]
async fn main() {
    let client = reqwest::Client::builder()
        .user_agent(concat!("WSight/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");

    println!("GET {API}");
    match client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            println!("  状态 {status}");
            match response.text().await {
                Ok(text) => {
                    println!("  长度 {} 字节", text.len());
                    let looks_like_release = text.contains("\"tag_name\"");
                    println!("  含 tag_name 字段: {looks_like_release}");
                    println!("\n结论：请求通了。");
                }
                Err(e) => println!("\n结论：拿到响应但读不出正文：{e}"),
            }
        }
        Err(e) => {
            println!("  失败：{e}");
            println!("\n结论：连不上。上面这条里如果提到 certificate / tls，就是根证书问题。");
        }
    }
}
