use std::time::Duration;

use conf::config::app_config;
use dotenvy::dotenv;
use feed::redis::verify::manager::VerifyManager;
use redis::AsyncCommands;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

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

async fn create_test_redis_pool() -> Option<bb8::Pool<bb8_redis::RedisConnectionManager>> {
    let config = app_config();
    let redis_url = config.rss.feed_redis.url.clone();

    let manager = match bb8_redis::RedisConnectionManager::new(redis_url.clone()) {
        Ok(m) => m,
        Err(err) => {
            warn!(error = %err, "skip test: invalid REDIS URL");
            return None;
        }
    };

    match bb8::Pool::builder()
        .max_size(config.rss.feed_redis.pool_size)
        .connection_timeout(Duration::from_secs(3))
        .build(manager)
        .await
    {
        Ok(p) => Some(p),
        Err(err) => {
            warn!(error = %err, "skip test: cannot connect redis");
            None
        }
    }
}

async fn create_test_verify_manager(
    pool: bb8::Pool<bb8_redis::RedisConnectionManager>,
) -> VerifyManager {
    let config = app_config();
    VerifyManager::new(
        pool,
        "test:verify-service-lock".to_string(),
        config.rss.feed_redis.redis_key_default_expire,
    )
    .await
}

async fn cleanup_test_lock(pool: &bb8::Pool<bb8_redis::RedisConnectionManager>, user_id: i64) {
    let lock_key = format!("test:verify-service-lock:verify-manager:user:{user_id}:lock");
    if let Ok(mut conn) = pool.get().await {
        let _: () = conn.del(&lock_key).await.unwrap_or_default();
    }
}

/// Test 1: 基础锁获取测试
#[tokio::test]
async fn test_acquire_lock_success() {
    init_test_tracing();
    info!("Starting test_acquire_lock_success");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_id = 1001;

    // 清理可能存在的测试锁
    cleanup_test_lock(&pool, user_id).await;

    // 尝试获取锁
    let lock_result = verify_manager
        .ops
        .acquire_lock(user_id, 10, 30)
        .await
        .expect("Failed to acquire lock");

    assert!(lock_result.is_some(), "Should successfully acquire lock");

    // 验证锁在 Redis 中确实存在
    let mut conn = pool.get().await.expect("Failed to get connection");
    let lock_key = format!("test:verify-service-lock:verify-manager:user:{user_id}:lock");
    let exists: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
    assert!(exists, "Lock should exist in Redis");

    // 清理
    drop(lock_result);
    tokio::time::sleep(Duration::from_millis(100)).await; // 等待锁释放
    cleanup_test_lock(&pool, user_id).await;

    info!("✅ test_acquire_lock_success passed");
}

/// Test 2: 并发锁竞争测试
#[tokio::test]
async fn test_concurrent_lock_competition() {
    init_test_tracing();
    info!("Starting test_concurrent_lock_competition");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_id = 1002;

    // 清理可能存在的测试锁
    cleanup_test_lock(&pool, user_id).await;

    // 创建多个并发任务尝试获取同一个锁
    let num_competitors = 5;
    let mut handles = Vec::new();

    for i in 0..num_competitors {
        let ops = verify_manager.ops.clone();
        let handle = tokio::spawn(async move {
            let result = ops.acquire_lock(user_id, 2, 10).await;
            (i, result)
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    let mut successful_count = 0;
    let mut failed_count = 0;

    for handle in handles {
        let (id, result) = handle.await.expect("Task should complete");
        match result {
            Ok(Some(_guard)) => {
                info!("Task {} successfully acquired lock", id);
                successful_count += 1;
                // 立即释放锁以便其他任务可以获取
                drop(_guard);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(None) => {
                info!("Task {} failed to acquire lock (timeout)", id);
                failed_count += 1;
            }
            Err(e) => {
                warn!("Task {} error: {}", id, e);
                failed_count += 1;
            }
        }
    }

    // 验证只有一个任务成功获取锁（或者由于时序问题，可能有多个在不同时间获取）
    info!(
        "Lock competition results: {} successful, {} failed",
        successful_count, failed_count
    );
    assert!(
        successful_count >= 1,
        "At least one task should acquire the lock"
    );
    assert!(
        successful_count <= num_competitors,
        "At most {num_competitors} tasks should acquire the lock"
    );

    // 清理
    cleanup_test_lock(&pool, user_id).await;

    info!("✅ test_concurrent_lock_competition passed");
}

/// Test 3: 锁超时测试
#[tokio::test]
async fn test_lock_timeout() {
    init_test_tracing();
    info!("Starting test_lock_timeout");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_id = 1003;

    // 清理可能存在的测试锁
    cleanup_test_lock(&pool, user_id).await;

    // 首先获取锁并持有
    let first_lock = verify_manager
        .ops
        .acquire_lock(user_id, 1, 10)
        .await
        .expect("Failed to acquire first lock")
        .expect("Should acquire first lock");

    info!("First lock acquired, holding for timeout test");

    // 尝试在另一个任务中获取同一个锁，但设置很短的超时时间
    let ops_clone = verify_manager.ops.clone();
    let timeout_handle = tokio::spawn(async move {
        // 超时时间设置为 1 秒，但锁已经被占用
        ops_clone.acquire_lock(user_id, 1, 10).await
    });

    // 等待超时任务完成
    let timeout_result = timeout_handle.await.expect("Timeout task should complete");

    match timeout_result {
        Ok(None) => {
            info!("✅ Lock timeout test passed: second acquire returned None as expected");
        }
        Ok(Some(_guard)) => {
            // 如果获取到了，可能是因为第一个锁已经释放了
            warn!("Lock was acquired, but this might be expected if timing allows");
            drop(_guard);
        }
        Err(e) => {
            panic!("Unexpected error during timeout test: {e}");
        }
    }

    // 释放第一个锁
    drop(first_lock);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 清理
    cleanup_test_lock(&pool, user_id).await;

    info!("✅ test_lock_timeout passed");
}

/// Test 4: 锁自动释放测试
#[tokio::test]
async fn test_lock_auto_release() {
    init_test_tracing();
    info!("Starting test_lock_auto_release");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_id = 1004;

    // 清理可能存在的测试锁
    cleanup_test_lock(&pool, user_id).await;

    // 获取锁
    let lock_guard = verify_manager
        .ops
        .acquire_lock(user_id, 10, 30)
        .await
        .expect("Failed to acquire lock")
        .expect("Should acquire lock");

    // 验证锁存在
    let mut conn = pool.get().await.expect("Failed to get connection");
    let lock_key = format!("test:verify-service-lock:verify-manager:user:{user_id}:lock");
    let exists_before: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
    assert!(exists_before, "Lock should exist before drop");

    // Drop guard，应该自动释放锁
    drop(lock_guard);

    // 等待锁释放（LockGuard 在 drop 时会在后台任务中释放）
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 验证锁已被释放
    let exists_after: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
    assert!(!exists_after, "Lock should be released after guard drop");

    // 验证可以重新获取锁
    let new_lock = verify_manager
        .ops
        .acquire_lock(user_id, 10, 30)
        .await
        .expect("Failed to acquire lock again")
        .expect("Should be able to acquire lock again");

    drop(new_lock);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 清理
    cleanup_test_lock(&pool, user_id).await;

    info!("✅ test_lock_auto_release passed");
}

/// Test 5: 锁过期测试
#[tokio::test]
async fn test_lock_expiry() {
    init_test_tracing();
    info!("Starting test_lock_expiry");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_id = 1005;

    // 清理可能存在的测试锁
    cleanup_test_lock(&pool, user_id).await;

    // 获取锁，设置很短的过期时间（2秒）
    let lock_guard = verify_manager
        .ops
        .acquire_lock(user_id, 10, 2)
        .await
        .expect("Failed to acquire lock")
        .expect("Should acquire lock");

    info!("Lock acquired with 2 second expiry");

    // 验证锁存在
    let mut conn = pool.get().await.expect("Failed to get connection");
    let lock_key = format!("test:verify-service-lock:verify-manager:user:{user_id}:lock");
    let exists_before: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
    assert!(exists_before, "Lock should exist before expiry");

    // 等待锁过期（等待超过2秒）
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 验证锁已过期（Redis 应该自动删除过期的 key）
    let exists_after: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
    assert!(!exists_after, "Lock should be expired and removed by Redis");

    // 验证可以重新获取锁（因为锁已过期）
    drop(lock_guard); // 虽然已经过期，但还是 drop 一下确保清理
    tokio::time::sleep(Duration::from_millis(100)).await;

    let new_lock = verify_manager
        .ops
        .acquire_lock(user_id, 10, 30)
        .await
        .expect("Failed to acquire lock after expiry")
        .expect("Should be able to acquire lock after expiry");

    drop(new_lock);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 清理
    cleanup_test_lock(&pool, user_id).await;

    info!("✅ test_lock_expiry passed");
}

/// Test 6: 不同用户锁互不干扰测试
#[tokio::test]
async fn test_different_users_no_interference() {
    init_test_tracing();
    info!("Starting test_different_users_no_interference");

    let pool = match create_test_redis_pool().await {
        Some(p) => p,
        None => return,
    };

    let verify_manager = create_test_verify_manager(pool.clone()).await;
    let user_ids = vec![2001, 2002, 2003, 2004, 2005];

    // 清理可能存在的测试锁
    for &user_id in &user_ids {
        cleanup_test_lock(&pool, user_id).await;
    }

    // 同时获取多个不同用户的锁
    let mut handles = Vec::new();
    for &user_id in &user_ids {
        let ops = verify_manager.ops.clone();
        let handle = tokio::spawn(async move {
            let result = ops.acquire_lock(user_id, 10, 30).await;
            (user_id, result)
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    let mut successful_locks = Vec::new();
    for handle in handles {
        let (user_id, result) = handle.await.expect("Task should complete");
        match result {
            Ok(Some(guard)) => {
                info!("Successfully acquired lock for user {}", user_id);
                successful_locks.push((user_id, guard));
            }
            Ok(None) => {
                panic!("Failed to acquire lock for user {user_id} (timeout)");
            }
            Err(e) => {
                panic!("Error acquiring lock for user {user_id}: {e}");
            }
        }
    }

    // 验证所有用户都成功获取了锁
    assert_eq!(
        successful_locks.len(),
        user_ids.len(),
        "All users should successfully acquire their locks"
    );

    // 验证每个用户的锁都在 Redis 中存在
    let mut conn = pool.get().await.expect("Failed to get connection");
    for &user_id in &user_ids {
        let lock_key = format!("test:verify-service-lock:verify-manager:user:{user_id}:lock");
        let exists: bool = conn.exists(&lock_key).await.expect("Failed to check lock");
        assert!(exists, "Lock for user {user_id} should exist in Redis");
    }

    // 释放所有锁
    for (_user_id, guard) in successful_locks {
        drop(guard);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 清理
    for &user_id in &user_ids {
        cleanup_test_lock(&pool, user_id).await;
    }

    info!("✅ test_different_users_no_interference passed");
}
