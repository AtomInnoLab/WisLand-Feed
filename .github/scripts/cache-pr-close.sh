#!/usr/bin/env bash
# PR 关闭时清理 GHA cache：
# 1. 删除该 PR merge ref 的 cache
# 2. 若仓库总量仍 > THRESHOLD_GB，按 last_accessed_at 从旧到新删至阈值以下
#
# 认证（gh CLI）：
#   - GitHub Actions：由 workflow 注入 GH_TOKEN=${{ github.token }}，无需 gh auth login
#   - 本地调试：先 gh auth login，并 export GITHUB_REPOSITORY=owner/repo
# 依赖环境变量：GITHUB_REPOSITORY（Actions 自动提供）、GH_TOKEN 或 GITHUB_TOKEN
set -euo pipefail

THRESHOLD_GB="${THRESHOLD_GB:-8}"
THRESHOLD_BYTES=$((THRESHOLD_GB * 1024 * 1024 * 1024))
REPO="${GITHUB_REPOSITORY:?}"
PR_NUMBER="${1:-}"

bytes_to_gb() {
  awk -v b="$1" 'BEGIN { printf "%.2f", b / 1024 / 1024 / 1024 }'
}

sum_repo_cache_bytes() {
  local total=0 page=1 count page_sum
  while true; do
    count=$(gh api "repos/${REPO}/actions/caches?per_page=100&page=${page}" --jq '.actions_caches | length')
    [[ "$count" -eq 0 ]] && break
    page_sum=$(gh api "repos/${REPO}/actions/caches?per_page=100&page=${page}" \
      --jq '[.actions_caches[].size_in_bytes] | add // 0')
    total=$((total + page_sum))
    [[ "$count" -lt 100 ]] && break
    page=$((page + 1))
  done
  echo "$total"
}

echo "=== cache-pr-close: ${REPO} threshold=${THRESHOLD_GB}GB ==="

if [[ -n "$PR_NUMBER" ]]; then
  BRANCH="refs/pull/${PR_NUMBER}/merge"
  echo "Delete caches for ${BRANCH}"
  set +e
  ids=$(gh cache list --ref "$BRANCH" --limit 100 --json id --jq '.[].id' 2>/dev/null || true)
  for id in $ids; do
    gh cache delete "$id" 2>/dev/null || true
  done
  set -e
fi

total=$(sum_repo_cache_bytes)
echo "Repo cache total: $(bytes_to_gb "$total")GB"

if [[ "$total" -le "$THRESHOLD_BYTES" ]]; then
  echo "Under threshold, skip LRU trim."
  exit 0
fi

echo "Over ${THRESHOLD_GB}GB — trim oldest caches by last_accessed_at..."
list_file=$(mktemp)
trap 'rm -f "$list_file"' EXIT

page=1
while true; do
  gh api "repos/${REPO}/actions/caches?per_page=100&page=${page}" \
    --jq '.actions_caches[] | "\(.last_accessed_at // .created_at) \(.id) \(.size_in_bytes)"' >>"$list_file" || true
  count=$(gh api "repos/${REPO}/actions/caches?per_page=100&page=${page}" --jq '.actions_caches | length')
  [[ "$count" -eq 0 ]] && break
  [[ "$count" -lt 100 ]] && break
  page=$((page + 1))
done

sort -o "$list_file" "$list_file"

current=$total
deleted=0
while read -r _la id sz; do
  [[ -z "${id:-}" ]] && continue
  if [[ "$current" -le "$THRESHOLD_BYTES" ]]; then
    break
  fi
  if gh cache delete "$id" 2>/dev/null; then
    current=$((current - sz))
    deleted=$((deleted + 1))
  fi
done <"$list_file"

echo "Trim done: deleted=${deleted}, approx remaining=$(bytes_to_gb "$current")GB"
