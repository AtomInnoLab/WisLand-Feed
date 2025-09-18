use std::io::{BufRead, Cursor};

use conf::config::app_config;
use feed::parsers::{arxiv::ArxivParser, base::Parser};
use std::{sync::Arc, time::Duration};
use tracing::info;
use tracing_subscriber::EnvFilter;

use oss::client::OssClient;
static INIT_TRACING: std::sync::Once = std::sync::Once::new();

fn init_test_tracing() {
    INIT_TRACING.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .compact()
            .try_init();
    });
}

#[tokio::test]
async fn test_arxiv_parser_minimal_rss_ok() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let url = "arxiv/10264175983241631030.xml";

    let oss_config = app_config().oss.clone();
    info!(?oss_config, "downloading RSS from OSS");
    let oss_client = OssClient::new(&oss_config)?;

    let xml_buf = oss_client.download(url).await?;
    let mut reader = std::io::Cursor::new(xml_buf.to_vec());
    let parser = ArxivParser {};
    let result = parser.parse(1, 1, "arxiv".to_string(), &mut reader);
    assert!(
        result.is_ok(),
        "ArxivParser should parse minimal RSS successfully"
    );
    let papers = result.unwrap();
    info!(count = papers.len(), "ArxivParser returned papers vector");
    assert_eq!(papers.len(), 0, "Current implementation returns empty list");
    Ok(())
}

#[tokio::test]
async fn test_arxiv_parser_invalid_input_err() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let mut cursor = Cursor::new(b"not xml".as_slice());
    let parser = ArxivParser {};
    let result = parser.parse(1, 1, "arxiv".to_string(), &mut cursor);
    assert!(result.is_err(), "ArxivParser should error on invalid input");
    Ok(())
}
