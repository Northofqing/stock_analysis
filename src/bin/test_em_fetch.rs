#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let client = reqwest::Client::new();
    let url = "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid=0.002092&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&klt=101&fqt=1&end=20500101&lmt=250";
    let resp = client
        .get(url)
        // REMOVE ENCODING GZIP TO PREVENT CORRUPTION OR USE reqwest::ClientBuilder::new().gzip(true).build()
        // .header("Accept-Encoding", "gzip, deflate, br")
        .send()
        .await
        .unwrap();
    let text = resp.text().await.unwrap();
    println!("First 50 chars: {}", &text[..50]);
}
