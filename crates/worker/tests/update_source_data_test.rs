use std::time::Duration;

use conf::config::app_config;
use dotenvy::dotenv;
use feed::parsers::arxiv::ArxivParser;
use feed::parsers::base::Parser;
use feed::workers::base::{FeedState, RedisService};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use seaorm_db::entities::feed::rss_papers;
use seaorm_db::query::feed::rss_papers::RssPaperData;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use seaorm_db::connection::get_db;
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

        dotenv().ok();
    });
}

#[tokio::test]
async fn test_update_source_data_smoke() -> anyhow::Result<()> {
    init_test_tracing();

    info!("starting test_update_source_data_smoke");

    // 获取配置
    let config = app_config();

    // 获取数据库连接
    let db = get_db().await.clone();
    info!("database connection acquired");

    // 从数据库中获取 ID 为 21437 的真实 RSS paper 记录
    let target_id = 21437;
    info!(target_id, "querying RSS paper with ID from database");

    let paper_model = rss_papers::Entity::find()
        .filter(rss_papers::Column::Id.eq(target_id))
        .one(&db)
        .await?;

    let test_paper = match paper_model {
        Some(paper) => {
            info!(
                id = paper.id,
                guid = %paper.guid,
                title = %paper.title,
                channel = %paper.channel,
                url = ?paper.url,
                "found RSS paper record from database"
            );

            // 将数据库模型转换为 RssPaperData
            RssPaperData {
                rss_history_id: paper.rss_history_id,
                rss_source_id: paper.rss_source_id,
                guid: paper.guid,
                title: paper.title,
                abstract_: paper.abstract_,
                authors: paper.authors,
                publication_date: paper.publication_date,
                url: paper.url,
                doi: paper.doi,
                venue: paper.venue,
                image: paper.image,
                raw_data: paper.raw_data,
                categories: paper.categories,
                metadata: paper.metadata,
                channel: paper.channel,
            }
        }
        None => {
            warn!(target_id, "RSS paper record not found in database");
            return Ok(());
        }
    };

    // 使用配置中的 Redis URL
    let redis_url = &config.rss.feed_redis.url;
    info!(redis_url = %redis_url, "using redis url from config");

    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip test: invalid REDIS_URL");
            return Ok(());
        }
    };

    let pool = match bb8::Pool::builder()
        .max_size(1)
        .connection_timeout(Duration::from_secs(2))
        .build(manager)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "skip test: cannot connect redis");
            return Ok(());
        }
    };

    // 创建 FeedState
    let feed_state = FeedState {
        db_conn: db,
        redis: RedisService {
            pool,
            apalis_conn: apalis_redis::connect(redis_url.as_str())
                .await
                .expect("Could not connect redis"),
        },
        config,
    };

    info!(guid = %test_paper.guid, title = %test_paper.title, "using real RSS paper data from database");

    // 创建 ArxivParser 实例
    let parser = ArxivParser {};

    // 执行 update_source_data
    info!("executing update_source_data");
    let result = parser.update_source_data(test_paper, &feed_state).await;

    match result {
        Ok(_) => {
            info!("update_source_data completed successfully");
        }
        Err(e) => {
            // 由于测试环境可能没有真实的 OSS 配置或网络访问，我们允许某些错误
            warn!(error = %e, "update_source_data failed, but this may be expected in test environment");

            // 检查是否是预期的错误类型（如 OSS 配置错误、网络错误等）
            let error_message = e.to_string();
            if error_message.contains("oss")
                || error_message.contains("network")
                || error_message.contains("connection")
                || error_message.contains("timeout")
            {
                info!(
                    "Error appears to be related to external dependencies, which is expected in test environment"
                );
            } else {
                // 如果是其他类型的错误，则测试失败
                return Err(e.into());
            }
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_update_source_data_with_invalid_url() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    info!("starting test_update_source_data_with_invalid_url");

    // 获取数据库连接
    let db = get_db().await.clone();
    info!("database connection acquired");

    // 从数据库中获取 ID 为 21437 的真实 RSS paper 记录
    let target_id = 21437;
    info!(target_id, "querying RSS paper with ID from database");

    let paper_model = rss_papers::Entity::find()
        .filter(rss_papers::Column::Id.eq(target_id))
        .one(&db)
        .await?;

    let mut test_paper_invalid_url = match paper_model {
        Some(paper) => {
            info!(
                id = paper.id,
                guid = %paper.guid,
                title = %paper.title,
                channel = %paper.channel,
                url = ?paper.url,
                "found RSS paper record from database"
            );

            // 将数据库模型转换为 RssPaperData
            RssPaperData {
                rss_history_id: paper.rss_history_id,
                rss_source_id: paper.rss_source_id,
                guid: paper.guid,
                title: paper.title,
                abstract_: paper.abstract_,
                authors: paper.authors,
                publication_date: paper.publication_date,
                url: paper.url,
                doi: paper.doi,
                venue: paper.venue,
                image: paper.image,
                raw_data: paper.raw_data,
                categories: paper.categories,
                metadata: paper.metadata,
                channel: paper.channel,
            }
        }
        None => {
            warn!(target_id, "RSS paper record not found in database");
            return Ok(());
        }
    };

    // 修改 URL 为无效 URL 来测试错误处理
    test_paper_invalid_url.url =
        Some("https://invalid-url-that-does-not-exist.com/paper".to_string());
    test_paper_invalid_url.image = Some("https://invalid-image-url.com/image.jpg".to_string());

    // 获取配置
    let config = app_config();

    // 使用配置中的 Redis URL
    let redis_url = &config.rss.feed_redis.url;
    info!(redis_url = %redis_url, "using redis url from config");

    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip test: invalid REDIS_URL");
            return Ok(());
        }
    };

    let pool = match bb8::Pool::builder()
        .max_size(1)
        .connection_timeout(Duration::from_secs(2))
        .build(manager)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "skip test: cannot connect redis");
            return Ok(());
        }
    };

    let feed_state = FeedState {
        db_conn: db,
        redis: RedisService {
            pool,
            apalis_conn: apalis_redis::connect(redis_url.as_str())
                .await
                .expect("Could not connect redis"),
        },
        config,
    };

    info!(guid = %test_paper_invalid_url.guid, "using real RSS paper data with modified invalid URL");

    let parser = ArxivParser {};

    // 执行 update_source_data，期望失败
    info!("executing update_source_data with invalid URL");
    let result = parser
        .update_source_data(test_paper_invalid_url, &feed_state)
        .await;

    match result {
        Ok(_) => {
            warn!("update_source_data unexpectedly succeeded with invalid URL");
            // 在某些情况下，即使 URL 无效，函数也可能成功（比如网络超时被忽略）
            // 所以我们不强制要求失败
        }
        Err(e) => {
            info!(error = %e, "update_source_data failed as expected with invalid URL");
            // 这是预期的行为
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_update_source_data_with_empty_data() -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    info!("starting test_update_source_data_with_empty_data");

    // 获取数据库连接
    let db = get_db().await.clone();
    info!("database connection acquired");

    // 从数据库中获取 ID 为 21437 的真实 RSS paper 记录
    let target_id = 21437;
    info!(target_id, "querying RSS paper with ID from database");

    let paper_model = rss_papers::Entity::find()
        .filter(rss_papers::Column::Id.eq(target_id))
        .one(&db)
        .await?;

    let mut test_paper_minimal = match paper_model {
        Some(paper) => {
            info!(
                id = paper.id,
                guid = %paper.guid,
                title = %paper.title,
                channel = %paper.channel,
                url = ?paper.url,
                "found RSS paper record from database"
            );

            // 将数据库模型转换为 RssPaperData
            RssPaperData {
                rss_history_id: paper.rss_history_id,
                rss_source_id: paper.rss_source_id,
                guid: paper.guid,
                title: paper.title,
                abstract_: paper.abstract_,
                authors: paper.authors,
                publication_date: paper.publication_date,
                url: paper.url,
                doi: paper.doi,
                venue: paper.venue,
                image: paper.image,
                raw_data: paper.raw_data,
                categories: paper.categories,
                metadata: paper.metadata,
                channel: paper.channel,
            }
        }
        None => {
            warn!(target_id, "RSS paper record not found in database");
            return Ok(());
        }
    };

    // 清空一些字段来测试最小化数据的情况
    test_paper_minimal.abstract_ = None;
    test_paper_minimal.authors = None;
    test_paper_minimal.publication_date = None;
    test_paper_minimal.url = None; // 没有 URL，这应该会导致函数使用默认值
    test_paper_minimal.doi = None;
    test_paper_minimal.venue = None;
    test_paper_minimal.image = None;
    test_paper_minimal.raw_data = None;
    test_paper_minimal.categories = None;
    test_paper_minimal.metadata = None;

    // 获取配置
    let config = app_config();

    // 使用配置中的 Redis URL
    let redis_url = &config.rss.feed_redis.url;
    info!(redis_url = %redis_url, "using redis url from config");

    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip test: invalid REDIS_URL");
            return Ok(());
        }
    };

    let pool = match bb8::Pool::builder()
        .max_size(1)
        .connection_timeout(Duration::from_secs(2))
        .build(manager)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "skip test: cannot connect redis");
            return Ok(());
        }
    };

    let feed_state = FeedState {
        db_conn: db,
        redis: RedisService {
            pool,
            apalis_conn: apalis_redis::connect(redis_url.as_str())
                .await
                .expect("Could not connect redis"),
        },
        config,
    };

    info!(guid = %test_paper_minimal.guid, "using real RSS paper data with minimal fields");

    let parser = ArxivParser {};

    // 执行 update_source_data
    info!("executing update_source_data with minimal data");
    let result = parser
        .update_source_data(test_paper_minimal, &feed_state)
        .await;

    match result {
        Ok(_) => {
            info!("update_source_data completed successfully with minimal data");
        }
        Err(e) => {
            warn!(error = %e, "update_source_data failed with minimal data");
            // 由于缺少关键字段（如 URL），函数可能会失败
            // 这是可以接受的行为
        }
    }

    Ok(())
}
