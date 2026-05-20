use async_openai::{config::OpenAIConfig, config::Config};

fn main() {
    let cfg = OpenAIConfig::new().with_api_key("test").with_api_base("base");
    println!("url: {}, key: {}", cfg.api_base(), secrecy::ExposeSecret::expose_secret(cfg.api_key()));
}
