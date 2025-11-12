use std::io::Cursor;

use dotenvy::{dotenv, from_read};
use nacos_sdk::api::config::ConfigServiceBuilder;
use nacos_sdk::api::props::ClientProps;

#[derive(Debug)]
pub enum NacosConfigError {
    MissingEndpoint,
    MissingNamespace,
    BuildConfigService(String),
    FetchConfig(String),
    ParseConfig(String),
}

impl std::fmt::Display for NacosConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NacosConfigError::BuildConfigService(err) => {
                write!(f, "failed to build nacos config service: {err}")
            }
            NacosConfigError::FetchConfig(err) => {
                write!(f, "failed to fetch nacos config: {err}")
            }
            NacosConfigError::ParseConfig(err) => write!(f, "failed to parse nacos config: {err}"),
            NacosConfigError::MissingEndpoint => write!(f, "NACOS_ENDPOINT is not set"),
            NacosConfigError::MissingNamespace => write!(f, "NACOS_NAMESPACE is not set"),
        }
    }
}

impl std::error::Error for NacosConfigError {}

pub async fn load_env_from_nacos(
    data_id: &str,
    app_name: &str,
    group_name: &str,
) -> Result<(), NacosConfigError> {
    dotenv().ok();

    let endpoint =
        std::env::var("NACOS_ENDPOINT").map_err(|_| NacosConfigError::MissingEndpoint)?;
    let namespace_id =
        std::env::var("NACOS_NAMESPACE").map_err(|_| NacosConfigError::MissingNamespace)?;
    println!("Loading configuration from Nacos: endpoint={endpoint}, namespace={namespace_id}");

    let client_props = ClientProps::new()
        .server_addr(endpoint)
        .namespace(namespace_id)
        .app_name(app_name);

    let config_service = ConfigServiceBuilder::new(client_props)
        .build()
        .map_err(|e| NacosConfigError::BuildConfigService(e.to_string()))?;

    let config_resp = config_service
        .get_config(data_id.to_string(), group_name.to_string())
        .await
        .map_err(|e| NacosConfigError::FetchConfig(e.to_string()))?;

    println!("config_resp: {config_resp:?}");

    let mut cursor = Cursor::new(config_resp.content().as_bytes().to_vec());
    from_read(&mut cursor).map_err(|e| NacosConfigError::ParseConfig(e.to_string()))?;

    println!("Nacos configuration loaded and applied to environment variables");
    Ok(())
}
