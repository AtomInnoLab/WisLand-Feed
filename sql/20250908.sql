--- chat_sessions
CREATE TYPE "session_category" AS ENUM ('chat', 'search');

CREATE TABLE IF NOT EXISTS "chat_sessions" ( "id" bigserial NOT NULL PRIMARY KEY, "team_id" bigint, "title" varchar(255) NOT NULL, "is_active" bool DEFAULT TRUE NOT NULL, "created_by_user_id" bigint, "doc_id" bigint NULL, "category" session_category NOT NULL DEFAULT 'chat'::session_category, "metadata" jsonb DEFAULT '{}'::jsonb NOT NULL, "created_at" timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL, "updated_at" timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL );
CREATE INDEX "idx_chat_sessions_active" ON "chat_sessions" ("is_active");
CREATE INDEX "idx_chat_sessions_created_by" ON "chat_sessions" ("created_by_user_id");
CREATE INDEX "idx_chat_sessions_team" ON "chat_sessions" ("team_id");
CREATE INDEX "idx_chat_sessions_category" ON "chat_sessions" ("category");
CREATE INDEX "idx_chat_sessions_docid_session_id" ON "chat_sessions" ("doc_id", "team_id", "id");

BEGIN;
-- session id
SELECT setval('chat_sessions_id_seq', (SELECT COUNT(*) FROM chat_session));

-- chat_messages;

-- 1. 重命名原 user_id 列为 user_id_str
ALTER TABLE chat_messages RENAME COLUMN user_id TO user_id_str;
ALTER TABLE chat_messages ALTER COLUMN user_id_str DROP NOT NULL;
ALTER TABLE chat_messages ALTER COLUMN user_id_str SET DEFAULT '';

-- 2. 添加新的 user_id 列（BIGINT, 可为空）
ALTER TABLE chat_messages ADD COLUMN user_id BIGINT;

-- 3. 删除旧的索引（基于 user_id_str 的字符串索引）
--    原索引名：idx_chat_messages_user_id（或其他，根据实际）
DROP INDEX IF EXISTS idx_chat_messages_user_id;

update chat_messages set user_id = -1;

-- 4. 为新的 user_id (BIGINT) 创建新索引
CREATE INDEX idx_chat_messages_user_id ON chat_messages(user_id);

-- 5. 可选：为兼容性添加注释
COMMENT ON COLUMN chat_messages.user_id_str IS 'Legacy string user ID (e.g. "auth0|abc123")';
COMMENT ON COLUMN chat_messages.user_id IS 'New numeric user ID, prefer this when available';

--- shared_messages
-- 1. 重命名原 user_id 列为 user_id_str
ALTER TABLE shared_messages RENAME COLUMN user_id TO user_id_str;
ALTER TABLE shared_messages ALTER COLUMN user_id_str DROP NOT NULL;
ALTER TABLE shared_messages ALTER COLUMN user_id_str SET DEFAULT '';

-- 2. 添加新的 user_id 列，BIGINT，NOT NULL，DEFAULT = -1
ALTER TABLE shared_messages ADD COLUMN user_id BIGINT NOT NULL DEFAULT -1;

-- 3. 删除旧的基于 user_id_str 的索引（避免混淆）
--    原索引名：idx_shared_messages_user_id
DROP INDEX IF EXISTS idx_shared_messages_user_id;

-- 4. 为新的 user_id (BIGINT) 创建新索引
CREATE INDEX idx_shared_messages_user_id ON shared_messages(user_id);

-- 5. 可选：添加注释，便于团队理解
COMMENT ON COLUMN shared_messages.user_id_str IS 'Legacy string user ID (e.g. "auth0|abc123")';
COMMENT ON COLUMN shared_messages.user_id IS 'New numeric user ID. -1 = not migrated';

--- seaql_migrations
DELETE FROM seaql_migrations;
INSERT INTO seaql_migrations (version, applied_at) VALUES
    ('m20250403_100502_chat_sessions', 1756985879),
    ('m20250403_101144_chat_messages', 1756985879),
    ('m20250407_034149_chat_llm_completions', 1756985879),
    ('m20250826_083211_shared_history', 1756985879);
COMMIT;
