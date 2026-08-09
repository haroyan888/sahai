#!/usr/bin/env bash
# 差配(Sahai)の状態を初期化するスクリプト(Linux/Mac用)。
# セットアップをやり直せるよう、DB・設定・登録済みサービスを消す。
#
# 使い方:
#   ./clean.sh              確認プロンプトあり
#   ./clean.sh --yes        確認なしで実行
#   ./clean.sh --cli-config CLIの接続先設定(~/.config/sahai/config.toml)も消す
#   ./clean.sh --acme       取得済みのTLS証明書も消す(既定では残す)
#
# 設定ファイルだけを消してDBを残すと「セットアップ済みだがトークンが分からない」
# 状態になり復旧できないため、このスクリプトは常に一括で消す。
#
# ただしTLS証明書(traefik/acme)は既定で残す。Let's Encryptには「同じ識別子の組に
# 対して7日間で5枚」という発行上限があり、消すとやり直すたびに1枚消費して
# 数日間再取得できなくなるため。証明書は残っていてもセットアップをやり直せる。
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

COMPOSE_FILE="$SCRIPT_DIR/compose.yaml"
DATA_ROOT="/var/sahai"
SETUP_ENV="$HOME/.config/sahai/setup.env"
CLI_CONFIG="$HOME/.config/sahai/config.toml"
HTPASSWD_FILE="$SCRIPT_DIR/registry/auth/htpasswd"
LEGACY_ENV_FILE="$SCRIPT_DIR/.env"

log() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { warn "エラー: $*"; exit 1; }

assume_yes=0
remove_cli_config=0
remove_acme=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) assume_yes=1 ;;
    --cli-config) remove_cli_config=1 ;;
    --acme) remove_acme=1 ;;
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明なオプション: $arg" ;;
  esac
done

command -v docker >/dev/null 2>&1 || die "Dockerが見つかりません。"
docker info >/dev/null 2>&1 || die "Dockerデーモンに接続できません。"

log "以下を削除します:"
log "  - 土台のコンテナ(traefik / sahai-server / registry)"
log "  - sahaiが起動した全サービスのコンテナ(svc-*)"
log "  - ${DATA_ROOT} 配下(DB・レジストリのイメージ・サービスのボリューム)"
log "  - ${HTPASSWD_FILE}"
log "  - ${SETUP_ENV}"
[ "$remove_cli_config" = 1 ] && log "  - ${CLI_CONFIG}"
[ "$remove_acme" = 1 ] && log "  - ${DATA_ROOT}/traefik/acme(取得済みのTLS証明書)"
log ""
log "サービスの永続化データも消えます。元に戻せません。"
if [ "$remove_acme" != 1 ]; then
  log "TLS証明書(${DATA_ROOT}/traefik/acme)は残します。Let's Encryptの発行上限を"
  log "使い切らないためです。消す場合は --acme を付けてください。"
fi

if [ "$assume_yes" != 1 ]; then
  read -r -p "続行しますか? [y/N]: " answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) log "中止しました。"; exit 0 ;;
  esac
fi

# 1. sahaiが起動したサービスのコンテナ(compose型はプロジェクトごと消えるようネットワークも)
log "サービスのコンテナを削除しています..."
svc_containers="$(docker ps -aq --filter 'name=^svc-' || true)"
[ -n "$svc_containers" ] && docker rm -f $svc_containers >/dev/null || true
svc_networks="$(docker network ls -q --filter 'name=^svc-' || true)"
[ -n "$svc_networks" ] && docker network rm $svc_networks >/dev/null 2>&1 || true

# 2. 土台のコンテナ
if [ -f "$COMPOSE_FILE" ]; then
  log "土台のコンテナを停止しています..."
  docker compose -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
fi

# 3. データルート。root所有かつ700のためホストから直接消せず、コンテナ経由で削除する
if docker run --rm -v "$DATA_ROOT:/target" alpine test -d /target >/dev/null 2>&1; then
  log "${DATA_ROOT} を削除しています..."
  if [ "$remove_acme" = 1 ]; then
    docker run --rm -v "$DATA_ROOT:/target" alpine sh -c 'rm -rf /target/..?* /target/.[!.]* /target/*' >/dev/null 2>&1 || true
  else
    # traefik/acmeだけ残す。findで1階層ずつ除外する
    docker run --rm -v "$DATA_ROOT:/target" alpine sh -c '
      find /target -mindepth 1 -maxdepth 1 ! -name traefik -exec rm -rf {} +
      [ -d /target/traefik ] && find /target/traefik -mindepth 1 -maxdepth 1 ! -name acme -exec rm -rf {} +
      exit 0
    ' >/dev/null 2>&1 || true
  fi
fi

# 4. リポジトリ内・ホーム配下のファイル
rm -f "$HTPASSWD_FILE" "$SETUP_ENV"
[ "$remove_cli_config" = 1 ] && rm -f "$CLI_CONFIG"
if [ -f "$LEGACY_ENV_FILE" ]; then
  warn "注意: 旧バージョンが作った ${LEGACY_ENV_FILE} が残っています。秘匿値を含むため内容を確認のうえ削除してください。"
fi

log ""
log "初期化しました。./setup.sh でセットアップし直せます。"
if [ "$remove_cli_config" != 1 ] && [ -f "$CLI_CONFIG" ]; then
  log "CLIの設定(${CLI_CONFIG})は残しています。再セットアップ後はAPIトークンが変わるため 'sahai login' をやり直してください。"
fi
