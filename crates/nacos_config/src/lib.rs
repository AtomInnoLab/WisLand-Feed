use std::io::Cursor;

use dotenvy::from_read;
use nacos_sdk::api::config::ConfigServiceBuilder;
use nacos_sdk::api::props::ClientProps;

#[derive(Debug)]
pub enum NacosConfigError {
    UnsupportedEnvironment(String),
    BuildConfigService(String),
    FetchConfig(String),
    ParseConfig(String),
}

impl std::fmt::Display for NacosConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NacosConfigError::UnsupportedEnvironment(env) => {
                write!(f, "unsupported environment: {env}")
            }
            NacosConfigError::BuildConfigService(err) => {
                write!(f, "failed to build nacos config service: {err}")
            }
            NacosConfigError::FetchConfig(err) => {
                write!(f, "failed to fetch nacos config: {err}")
            }
            NacosConfigError::ParseConfig(err) => write!(f, "failed to parse nacos config: {err}"),
        }
    }
}

impl std::error::Error for NacosConfigError {}

pub async fn load_env_from_nacos(
    data_id: &str,
    app_name: &str,
    group_name: &str,
) -> Result<(), NacosConfigError> {
    let env_raw = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "local".to_string());
    let environment = env_raw.to_lowercase();
    println!("Loading configuration from Nacos for environment: {environment}");

    let namespace_id = match environment.as_str() {
        "local" => "28452470-afb0-4698-bd51-ad8508f84798",
        "dev" => "8d222d2a-b3f7-4229-b44d-e8b305f9f512",
        "pre" => "918b7045-4408-474d-8cb5-541ff94e5584",
        "prod" => "your-prod-namespace-id",
        other => return Err(NacosConfigError::UnsupportedEnvironment(other.to_string())),
    };

    let server_addr = match environment.as_str() {
        "local" => "mse-9996a1110-p.nacos-ans.mse.aliyuncs.com:8848",
        _ => "mse-9996a1110-nacos-ans.mse.aliyuncs.com:8848",
    };

    let client_props = ClientProps::new()
        .server_addr(server_addr)
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
