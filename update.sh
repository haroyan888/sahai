#!/usr/bin/env bash
# 差配(Sahai)を最新版へ更新するスクリプト(Linux/Mac用)。
#
# 使い方:
#   ./update.sh              確認プロンプトあり
#   ./update.sh --yes        確認なしで実行
#   ./update.sh --no-pull    git pullを省き、手元のソースのまま再構築する
#
# 設定・証明書・サービスの永続化データには一切触らない。土台の3コンテナ
# (traefik / sahai-server / registry)を新しいイメージで作り直すだけ。
#
# 更新中も登録済みサービス(svc-*)は動き続ける。これらはcomposeの管理外であり、
# 土台の作り直しでは停止しないため。止まるのは管理画面とレジストリだけ。
#
# 起動時にDBマイグレーションが走る。失敗すると起動できず、SQLiteには
# ロールバックが無いため、事前にDBのバックアップを取る。
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

COMPOSE_FILE="$SCRIPT_DIR/compose.yaml"
DATA_ROOT="/var/sahai"
DB_PATH="db/sahai.sqlite3"
BACKUP_DIR="backups"
# 残す世代数。無制限に増やすとディスクを圧迫する
KEEP_BACKUPS=5

log() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { warn "エラー: $*"; exit 1; }

dc() { docker compose -f "$COMPOSE_FILE" "$@"; }

# DATA_ROOTはroot所有かつ700のためホストから直接読めない。コンテナ経由で操作する
in_data_root() { docker run --rm -v "$DATA_ROOT:/data" alpine sh -c "$1"; }

assume_yes=0
do_pull=1
for arg in "$@"; do
  case "$arg" in
    --yes|-y) assume_yes=1 ;;
    --no-pull) do_pull=0 ;;
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明なオプション: $arg" ;;
  esac
done

# ============================================================
# 0. 前提条件チェック
# ============================================================
step0_check_prerequisites() {
  command -v docker >/dev/null 2>&1 || die "Dockerが見つかりません。"
  docker compose version >/dev/null 2>&1 || die "docker compose v2が見つかりません。"
  docker info >/dev/null 2>&1 || die "Dockerデーモンに接続できません。"
  [ -f "$COMPOSE_FILE" ] || die "compose.yamlが見つかりません: $COMPOSE_FILE"

  # 未セットアップの環境で実行しても意味がないため止める
  if ! in_data_root "test -f /data/$DB_PATH" >/dev/null 2>&1; then
    die "セットアップがまだ完了していません(${DATA_ROOT}/${DB_PATH} が無い)。先に ./setup.sh を実行してください。"
  fi

  if [ "$do_pull" = 1 ]; then
    command -v git >/dev/null 2>&1 || die "gitが見つかりません。--no-pull を付ければgitなしで再構築できます。"
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 \
      || die "gitリポジトリではありません。--no-pull を付けてください。"
    # 未コミットの変更があるとpullが中断し、中途半端な状態になりうる
    if [ -n "$(git status --porcelain)" ]; then
      die "未コミットの変更があります。退避するか、--no-pull を付けて実行してください。"
    fi
  fi
}

# ============================================================
# 1. ソースの更新
# ============================================================
step1_pull_source() {
  if [ "$do_pull" != 1 ]; then
    log "git pullを省略します(--no-pull)。"
    return
  fi
  local before after
  before="$(git rev-parse --short HEAD)"
  log "リポジトリを更新しています..."
  # マージコミットを作らせない。作られると次回のpullが複雑になる
  git pull --ff-only || die "git pullに失敗しました。手動で解決してください。"
  after="$(git rev-parse --short HEAD)"

  if [ "$before" = "$after" ]; then
    log "  既に最新です($after)。"
  else
    log "  $before → $after"
    git --no-pager log --oneline "${before}..${after}" | sed 's/^/    /'
  fi
}

# ============================================================
# 2. DBのバックアップ
# ============================================================
step2_backup_database() {
  local stamp
  stamp="$(date +%Y%m%d-%H%M%S)"
  log "DBをバックアップしています..."
  # sahai-serverを止めてからコピーする。稼働中のSQLiteをそのままコピーすると
  # 書き込み途中の状態を掴む可能性がある。traefikは止めないため、
  # 登録済みサービスへのアクセスは維持される
  dc stop sahai-server >/dev/null 2>&1 || true
  in_data_root "mkdir -p /data/$BACKUP_DIR && cp /data/$DB_PATH /data/$BACKUP_DIR/sahai-$stamp.sqlite3" \
    || die "DBのバックアップに失敗しました。"
  log "  ${DATA_ROOT}/${BACKUP_DIR}/sahai-${stamp}.sqlite3"

  # 古い世代を削除する。新しい順に並べてKEEP_BACKUPS個より後ろを消す
  in_data_root "ls -1t /data/$BACKUP_DIR/sahai-*.sqlite3 2>/dev/null | tail -n +$((KEEP_BACKUPS + 1)) | xargs -r rm -f" \
    >/dev/null 2>&1 || true
}

# ============================================================
# 3. イメージの取得(取得できなければビルド)
# ============================================================
step3_pull_or_build_image() {
  # setup.shと同じ方針。`up --pull always`の暗黙のフォールバックには頼らない
  # (取得失敗時にビルドへ回るかはdocker composeのバージョンによって変わる)
  log "sahai-serverのイメージを取得しています..."
  if dc pull sahai-server >/dev/null 2>&1; then
    log "  公開イメージを取得しました。"
  else
    log "  公開イメージを取得できませんでした。ソースからビルドします(数分かかります)..."
    dc build sahai-server || die "sahai-serverのビルドに失敗しました。"
  fi
}

# ============================================================
# 4. 起動
# ============================================================
step4_compose_up() {
  log "コンテナを起動しています..."
  # --force-recreateで必ず作り直す。`up -d`だけだと、ネットワーク名の変更のように
  # コンテナ設定が変わったのに再作成が走らず、古い設定を参照したまま起動を試みて
  # 失敗することがある(実際に踏んだ)。更新時はどのみち作り直すので副作用は無い。
  # compose.yamlでtraefik/registryのタグが変わっていれば、ここで自動的に取得される
  dc up -d --force-recreate
}

# ============================================================
# 5. 起動待ち
# ============================================================
step5_wait_for_sahai_server_ready() {
  log "sahai-serverの起動を待っています..."
  local timeout_s=120 elapsed=0
  until dc exec -T sahai-server curl -fsS "http://localhost:8080/api/setup" >/dev/null 2>&1; do
    sleep 2
    elapsed=$((elapsed + 2))
    if [ "$elapsed" -ge "$timeout_s" ]; then
      warn ""
      warn "sahai-serverが${timeout_s}秒以内に起動しませんでした。"
      warn "マイグレーションに失敗した可能性があります。ログを確認してください:"
      warn "  docker compose -f compose.yaml logs sahai-server"
      warn ""
      warn "DBを戻す場合は、直前のバックアップを書き戻してから再起動してください:"
      warn "  docker run --rm -v ${DATA_ROOT}:/data alpine sh -c 'ls -1t /data/${BACKUP_DIR}'"
      exit 1
    fi
  done
  log "sahai-serverが起動しました。"
}

# ============================================================
# 6. 結果表示
# ============================================================
step6_print_summary() {
  log ""
  log "====================================================="
  log "更新が完了しました。"
  log ""
  log "  版: $(git rev-parse --short HEAD 2>/dev/null || echo '(git管理外)')"
  log ""
  log "設定・証明書・サービスのデータは変更していません。"
  log "登録済みサービスは更新中も動き続けています。"
  log ""
  log "ルーティングの生成規則が変わった場合に備え、気になるサービスは"
  log "再起動しておくと確実です:"
  log "  sahai service restart <サービス名>"
  log "====================================================="
}

log "差配を更新します。"
log "  対象: 土台のコンテナ(traefik / sahai-server / registry)"
log "  更新中、管理画面とレジストリは一時的に停止します。"
log "  登録済みサービスは停止しません。"
log ""

if [ "$assume_yes" != 1 ]; then
  read -r -p "続行しますか? [y/N]: " answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) log "中止しました。"; exit 0 ;;
  esac
fi

step0_check_prerequisites
step1_pull_source
step2_backup_database
step3_pull_or_build_image
step4_compose_up
step5_wait_for_sahai_server_ready
step6_print_summary
