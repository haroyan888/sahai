#!/usr/bin/env bash
# 差配(Sahai)の初回セットアップ〜起動を自動化するスクリプト(Linux/Mac用)。
# 対象は本番用 compose.yaml のみ(dev.compose.yamlは対象外)。
#
# 使い方:
#   ./setup.sh
#
# 非対話実行したい場合は SAHAI_SETUP_NONINTERACTIVE=1 を設定し、
# 併せて必要な値(SAHAI_SETUP_DOMAIN 等)を環境変数で渡すこと。
# 詳細な環境変数一覧はこのファイル内のコメント、および docs を参照。
#
# 注意: デバッグ目的でも `set -x` を追加しないこと。パスワード・APIトークン・
# DNS認証情報がシェルトレースに出力されてしまう。
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

COMPOSE_FILE="$SCRIPT_DIR/compose.yaml"
# 再実行時にAPIトークンを再利用するためだけの控え。設定値の正はDB側にあり、
# ここには他の値を書かない。CLIの~/.config/sahai/config.tomlと同じ配置方針
ENV_FILE="$HOME/.config/sahai/setup.env"
# 旧バージョンがリポジトリ直下に作っていた同等ファイル。トークンの引き継ぎのみに使う
LEGACY_ENV_FILE="$SCRIPT_DIR/.env"
DATA_ROOT="/var/sahai"
# 生成物はすべてデータルート配下に集約する(container-design.md 3章)。
# ここはdockerdが作るroot所有のディレクトリなので、読み書きはコンテナ経由で行い
# setup.sh自体にsudoを要求しない
HTPASSWD_DIR="$DATA_ROOT/registry-auth"
# v0.1系までリポジトリ直下に置いていた同等ファイル。移行のためだけに参照する
LEGACY_HTPASSWD_FILE="$SCRIPT_DIR/registry/auth/htpasswd"

log() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { warn "エラー: $*"; exit 1; }

# htpasswdの置き場($HTPASSWD_DIR)はdockerdが作るroot所有のディレクトリのため、
# ホスト側のリダイレクトでは書き込めない。コンテナ内から読み書きしてsudoを不要にする
# (clean.shが$DATA_ROOTの削除で使っているのと同じ手)
htpasswd_exists() {
  docker run --rm -v "$HTPASSWD_DIR:/auth" httpd:2.4-alpine test -f /auth/htpasswd >/dev/null 2>&1
}

# 引数: <ユーザー名>。パスワードは標準入力から渡す
# (引数に置くとプロセス一覧やdocker inspectのCmdに平文で残るため)
write_htpasswd() {
  docker run --rm -i -v "$HTPASSWD_DIR:/auth" httpd:2.4-alpine \
    sh -c 'htpasswd -Bni "$1" > /auth/htpasswd' sh "$1"
}

# 空のhtpasswdを置く。ファイルが存在しないままbind mountするとDockerが
# ディレクトリを自動作成してしまい、registryコンテナの起動自体が壊れる
write_empty_htpasswd() {
  docker run --rm -v "$HTPASSWD_DIR:/auth" httpd:2.4-alpine sh -c ': > /auth/htpasswd'
}

# リポジトリ直下(旧)からデータルート配下(新)へhtpasswdを引き継ぐ。
# これをしないと、既存環境の更新時にregistryが認証ファイルを見失い
# 「htpasswd is missing, provisioning with default user」で勝手にランダムな
# 資格情報を作ってしまい、既存のdocker loginが通らなくなる
migrate_legacy_htpasswd() {
  [ -f "$LEGACY_HTPASSWD_FILE" ] || return 0
  htpasswd_exists && return 0
  if docker run --rm -i -v "$HTPASSWD_DIR:/auth" httpd:2.4-alpine \
       sh -c 'cat > /auth/htpasswd' < "$LEGACY_HTPASSWD_FILE"; then
    log "${LEGACY_HTPASSWD_FILE} を ${HTPASSWD_DIR}/htpasswd へ移行しました。"
    log "移行元のファイルは不要です。内容を確認のうえ削除してください。"
  else
    warn "htpasswdの移行に失敗しました。レジストリの認証が効かなくなる可能性があります。"
  fi
}

dc() { docker compose -f "$COMPOSE_FILE" "$@"; }

# sahai-serverコンテナ内部からlocalhost:8080を叩く。ポート非公開のため
# ホストから直接到達する手段が無く、かつDocker Desktop(Windows/Mac)では
# コンテナのブリッジIPにホストから直接到達できないことが多いため、
# `docker compose exec`経由でコンテナ内部から自分自身を叩く方式に統一する
# (Dockerfileのruntimeステージにはdocker-cliインストールの依存としてcurlが
# 含まれている)。
api_get() {
  dc exec -T sahai-server curl -fsS "$@"
}

api_post_body() {
  local path="$1" body="$2"
  shift 2
  printf '%s' "$body" | dc exec -T sahai-server curl -fsS -X POST "http://localhost:8080${path}" \
    -H 'Content-Type: application/json' -d @- "$@"
}

api_put_body() {
  local path="$1" body="$2"
  shift 2
  printf '%s' "$body" | dc exec -T sahai-server curl -fsS -X PUT "http://localhost:8080${path}" \
    -H 'Content-Type: application/json' -d @- "$@"
}

# --- setup.env読み書きユーティリティ(env_file.rsのupsertと同等の挙動をbashで再現) ---

env_get_from() {
  local file="$1" key="$2"
  [ -f "$file" ] || return 1
  grep -q "^${key}=" "$file" || return 1
  sed -n "s/^${key}=//p" "$file" | tail -n1
}

env_get() {
  env_get_from "$ENV_FILE" "$1"
}

upsert_env_var() {
  local key="$1" value="$2"
  local is_new_file=0
  [ -f "$ENV_FILE" ] || is_new_file=1

  if [ "$is_new_file" = 1 ]; then
    mkdir -p "$(dirname "$ENV_FILE")"
  fi

  if [ -f "$ENV_FILE" ] && grep -q "^${key}=" "$ENV_FILE"; then
    local tmp
    tmp="$(mktemp)"
    awk -v k="$key" -v v="$value" -F= 'BEGIN{OFS="="} $1==k{$0=k"="v} {print}' "$ENV_FILE" > "$tmp"
    mv "$tmp" "$ENV_FILE"
  else
    printf '%s=%s\n' "$key" "$value" >> "$ENV_FILE"
  fi

  if [ "$is_new_file" = 1 ]; then
    chmod 600 "$ENV_FILE"
  fi
}

# APIトークン・レジストリパスワード等、十分に堅牢なランダム値が必要な箇所で共通利用する。
generate_random_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    head -c 32 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 43
  fi
}

# ============================================================
# 0. 前提条件チェック
# ============================================================
step0_check_prerequisites() {
  command -v docker >/dev/null 2>&1 || die "Dockerが見つかりません: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 || die "docker compose v2が見つかりません。Docker Engine 20.10以降が必要です。"
  docker info >/dev/null 2>&1 || die "Dockerデーモンに接続できません。起動状態と、現在のユーザーがdockerグループに属しているか確認してください。"
  command -v jq >/dev/null 2>&1 || die "jqが見つかりません(例: apt install jq)。"
  if ! command -v openssl >/dev/null 2>&1; then
    warn "opensslが無いため、証明書の自動確認をスキップします。"
  fi
}

# ============================================================
# 1. レジストリ資格情報の決定(htpasswd作成 + PUT /api/settings/registry用の値を確定)
# ============================================================
# 値の決定はここで1回だけ行う(auto/manualの選択、URL/ユーザー名/パスワードの確定)。
# htpasswdファイルはdocker compose up(step4)より前に存在しなければregistryコンテナの
# 認証が機能しないため、ここで書き出す。一方PUT /api/settings/registryはAPIトークン
# 確定・サーバー起動より後でなければ呼べないため、DBへの登録はstep10で行う
# (ここで確定した値をSAHAI_REGISTRY_*_VALUEに保持して引き継ぐ)。
#
# 結果はSAHAI_REGISTRY_CREDENTIALS_MODEに記録する:
#   provisioned    - 今回新たに値を決定した(step10でDB登録する)
#   reuse-existing - 既存のhtpasswdを再利用した(平文パスワードが分からないためDB登録はしない)
#   skip           - SAHAI_SETUP_SKIP_REGISTRY_SETTINGS=1で丸ごとスキップした
step1_configure_registry() {
  migrate_legacy_htpasswd

  if htpasswd_exists; then
    log "${HTPASSWD_DIR}/htpasswd は既存のものを再利用します。"
    log "変更したい場合はWeb UIの「レジストリ設定」から行えます。"
    SAHAI_REGISTRY_CREDENTIALS_MODE="reuse-existing"
    return 0
  fi

  if [ "${SAHAI_SETUP_SKIP_REGISTRY_SETTINGS:-}" = "1" ]; then
    log "レジストリ設定をスキップしました(SAHAI_SETUP_SKIP_REGISTRY_SETTINGS=1)。"
    SAHAI_REGISTRY_CREDENTIALS_MODE="skip"
    return 0
  fi

  local mode
  if [ -n "${SAHAI_SETUP_REGISTRY_URL:-}${SAHAI_SETUP_REGISTRY_AUTH_USER:-}${SAHAI_SETUP_REGISTRY_AUTH_PASSWORD:-}" ]; then
    # 非対話向けの環境変数指定が1つでもあればmanual相当として扱う
    mode="manual"
  elif [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
    mode="auto"
  else
    log ""
    log "レジストリ(registry:2)の資格情報を設定します:"
    log "  1) auto(default)"
    log "  2) manual"
    local choice
    read -r -p "> " choice
    case "$choice" in
      2) mode="manual" ;;
      ""|1) mode="auto" ;;
      *) die "1または2を入力してください。" ;;
    esac
  fi

  local reg_url="" reg_user reg_pass reg_pass_confirm default_url
  if [ "$mode" = "manual" ]; then
    # URLが既定値(registry.sahai.<domain>)かどうかの判定にdomainが要るが、通常domainは
    # step6で初めて確定する(このstepより後)。ここで一度だけ確定させ、
    # SAHAI_SETUP_DOMAINとしてexportしておくことでstep6の二重プロンプトを防ぐ
    # (step6_run_initial_setup_if_neededは既にSAHAI_SETUP_DOMAINを優先して使う)。
    if [ -z "${SAHAI_SETUP_DOMAIN:-}" ]; then
      read -r -p "サービスのベースドメイン(例: example.com): " SAHAI_SETUP_DOMAIN
      [ -n "$SAHAI_SETUP_DOMAIN" ] || die "ドメインを入力してください。"
      log "  このホストを指すDNSレコードが2本必要です:"
      log "    *.${SAHAI_SETUP_DOMAIN}       (管理画面と各サービス)"
      log "    *.sahai.${SAHAI_SETUP_DOMAIN} (レジストリ)"
    fi
    export SAHAI_SETUP_DOMAIN
    default_url="registry.sahai.${SAHAI_SETUP_DOMAIN}"

    reg_url="${SAHAI_SETUP_REGISTRY_URL:-}"
    if [ -z "$reg_url" ]; then
      if [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
        reg_url="$default_url"
      else
        read -r -p "レジストリURL [${default_url}]: " reg_url
        reg_url="${reg_url:-$default_url}"
      fi
    fi

    reg_user="${SAHAI_SETUP_REGISTRY_AUTH_USER:-}"
    if [ -z "$reg_user" ]; then
      [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ] && die "非対話モードですが SAHAI_SETUP_REGISTRY_AUTH_USER が未設定です。"
      read -r -p "レジストリ用ユーザー名: " reg_user
      [ -n "$reg_user" ] || die "ユーザー名を入力してください。"
    fi

    reg_pass="${SAHAI_SETUP_REGISTRY_AUTH_PASSWORD:-}"
    if [ -z "$reg_pass" ]; then
      [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ] && die "非対話モードですが SAHAI_SETUP_REGISTRY_AUTH_PASSWORD が未設定です。"
      read -r -s -p "レジストリ用パスワード: " reg_pass; echo
      read -r -s -p "パスワード(確認): " reg_pass_confirm; echo
      [ "$reg_pass" = "$reg_pass_confirm" ] || die "パスワードが一致しません。"
    fi

    if [ "$reg_url" = "$default_url" ]; then
      printf '%s' "$reg_pass" | write_htpasswd "$reg_user"
      log "${HTPASSWD_DIR}/htpasswd を作成しました(ユーザー名: ${reg_user})。"
    else
      # 中身が空でも有効なhtpasswdファイルとして扱われ、単に誰も認証できなくなるだけ
      # で済む。この同梱レジストリは使わない前提なので問題ない
      write_empty_htpasswd
      log "外部レジストリを指定したため、同梱registryの認証ファイルは作成しません。"
    fi
  else
    # auto: ユーザー名固定+パスワード自動生成。URLは空のままサーバー側の
    # apply_registry_url_defaultにregistry.sahai.<domain>を自動補完させる
    reg_url=""
    reg_user="${SAHAI_SETUP_REGISTRY_AUTH_USER:-sahai}"
    reg_pass="${SAHAI_SETUP_REGISTRY_AUTH_PASSWORD:-$(generate_random_secret)}"
    printf '%s' "$reg_pass" | write_htpasswd "$reg_user"
    log "${HTPASSWD_DIR}/htpasswd を作成しました(ユーザー名: ${reg_user}、パスワードは自動生成)。"
  fi

  SAHAI_REGISTRY_CREDENTIALS_MODE="provisioned"
  SAHAI_REGISTRY_URL_VALUE="$reg_url"
  SAHAI_REGISTRY_AUTH_USER_VALUE="$reg_user"
  SAHAI_REGISTRY_AUTH_PASSWORD_VALUE="$reg_pass"
  unset reg_pass reg_pass_confirm
}

# ============================================================
# 2. SAHAI_API_TOKENの生成
# ============================================================
step2_ensure_api_token() {
  SAHAI_API_TOKEN_VALUE="$(env_get SAHAI_API_TOKEN || true)"
  if [ -n "$SAHAI_API_TOKEN_VALUE" ]; then
    log "既存のAPIトークンを再利用します。"
    return 0
  fi

  # 旧バージョンがリポジトリ直下の.envに保存していたトークンを引き継ぐ
  SAHAI_API_TOKEN_VALUE="$(env_get_from "$LEGACY_ENV_FILE" SAHAI_API_TOKEN || true)"
  if [ -n "$SAHAI_API_TOKEN_VALUE" ]; then
    log "既存のAPIトークンを ${LEGACY_ENV_FILE} から引き継ぎます。"
    LEGACY_ENV_FILE_MIGRATED=1
    return 0
  fi

  SAHAI_API_TOKEN_VALUE="$(generate_random_secret)"
}

# ============================================================
# 3. setup.envの作成/更新(SAHAI_API_TOKENのみ)
# ============================================================
step3_write_setup_env() {
  upsert_env_var SAHAI_API_TOKEN "$SAHAI_API_TOKEN_VALUE"
}

# ============================================================
# 4. 起動
# ============================================================
step4_compose_up_build() {
  # 公開イメージがあれば取得し、取得できなければソースからビルドする。
  # `up --pull always`の暗黙のフォールバックには頼らない。取得失敗時にビルドへ
  # 回るかどうかはdocker composeのバージョンによって変わり、古い版では
  # そのままエラー終了してしまうため。
  log "sahai-serverのイメージを取得しています..."
  if dc pull sahai-server >/dev/null 2>&1; then
    log "  公開イメージを取得しました。"
  else
    log "  公開イメージを取得できませんでした。ソースからビルドします(数分かかります)..."
    dc build sahai-server || die "sahai-serverのビルドに失敗しました。上のログを確認してください。"
  fi
  log "コンテナを起動しています..."
  dc up -d
}

# ============================================================
# 5. sahai-serverの起動待ち
# ============================================================
step5_wait_for_sahai_server_ready() {
  log "sahai-serverの起動を待っています..."
  local timeout_s=120 elapsed=0
  until api_get "http://localhost:8080/api/setup" >/dev/null 2>&1; do
    sleep 2
    elapsed=$((elapsed + 2))
    if [ "$elapsed" -ge "$timeout_s" ]; then
      die "sahai-serverが${timeout_s}秒以内に起動しませんでした。ログを確認してください: docker compose -f compose.yaml logs sahai-server"
    fi
  done
  log "sahai-serverが起動しました。"
}

# ============================================================
# 6. 初回セットアップ(POST /api/setup)
# ============================================================
step6_run_initial_setup_if_needed() {
  local configured
  configured="$(api_get 'http://localhost:8080/api/setup' | jq -r '.configured')"

  if [ "$configured" = "true" ]; then
    log "既にセットアップ済みのため初期設定はスキップします。"
    # setup.envの値ではなく、DBの正を都度取得する(setup.envはあくまで参考記録のため)。
    # SAHAI_API_TOKEN_VALUEがDB上のトークンと一致しない場合(DBのデータだけ残った
    # 状態でsetup.envを削除・再生成した場合等)、このAPI呼び出しは401で失敗する。
    # local var="$(cmd)"という書き方だと`local`自体の終了ステータスで上書きされ
    # `set -e`が失敗を検知できない(masking)ため、宣言と代入を分けて明示的に
    # `||`でハンドリングする。
    local settings_json
    settings_json="$(api_get 'http://localhost:8080/api/settings' -H "Authorization: Bearer $SAHAI_API_TOKEN_VALUE")" \
      || die "既にセットアップ済みですが、APIトークンでの認証に失敗しました。${ENV_FILE} のSAHAI_API_TOKENが、既存のDBに保存されているトークンと一致しているか確認してください(DBのデータだけ残ったままsetup.envを削除・再生成した場合等に発生します。正しいトークンが分からない場合は、Web UIから再発行するか、DBを含むデータをリセットして最初からセットアップし直してください)。"
    SAHAI_DOMAIN_VALUE="$(printf '%s' "$settings_json" | jq -r '.domain')"
    return 0
  fi

  local domain
  if [ -n "${SAHAI_SETUP_DOMAIN:-}" ]; then
    domain="$SAHAI_SETUP_DOMAIN"
  elif [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
    die "非対話モードですが SAHAI_SETUP_DOMAIN が未設定です。"
  else
    read -r -p "サービスのベースドメイン(例: example.com): " domain
    [ -n "$domain" ] || die "ドメインを入力してください。"
  fi

  local body
  body="$(jq -n --arg d "$domain" --arg t "$SAHAI_API_TOKEN_VALUE" \
    '{domain:$d, https_redirect:true, api_token:$t}')"

  # 初期設定の先取りを防ぐため、サーバーが起動時に発行したワンタイムトークンの提示が要る。
  # ファイルはSAHAI_DATA_ROOT(compose.yamlで/var/sahai固定)直下にあり、
  # ホストからは読めないためコンテナ内部から取得する
  local setup_token
  setup_token="$(dc exec -T sahai-server cat /var/sahai/setup-token 2>/dev/null | tr -d '\r\n')" \
    || die "セットアップトークンを取得できませんでした。ログを確認してください: docker compose -f compose.yaml logs sahai-server"
  [ -n "$setup_token" ] || die "セットアップトークンが空です。sahai-serverが未設定状態で起動しているか確認してください。"

  log "初期設定を保存しています..."
  api_post_body "/api/setup" "$body" -H "X-Sahai-Setup-Token: ${setup_token}" >/dev/null \
    || die "初期設定(POST /api/setup)に失敗しました。"
  unset setup_token

  SAHAI_DOMAIN_VALUE="$domain"
}

# ============================================================
# 7. DNS/証明書設定(PUT /api/settings/dns-provider)
# ============================================================
step7_configure_dns_and_tls() {
  local dns_provider="${SAHAI_DNS_PROVIDER:-}"
  if [ -z "$dns_provider" ]; then
    if [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
      die "非対話モードですが SAHAI_DNS_PROVIDER が未設定です。"
    fi
    read -r -p "DNSプロバイダ(legoが対応するプロバイダ名。例: cloudflare。一覧: https://go-acme.github.io/lego/dns/index.html): " dns_provider
    [ -n "$dns_provider" ] || die "DNSプロバイダを入力してください。"
  fi

  local acme_email="${SAHAI_ACME_EMAIL:-}"
  if [ -z "$acme_email" ]; then
    if [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
      die "非対話モードですが SAHAI_ACME_EMAIL が未設定です。"
    fi
    read -r -p "Let's Encrypt通知先メールアドレス: " acme_email
    [ -n "$acme_email" ] || die "メールアドレスを入力してください。"
  fi

  local credentials_json="[]"
  if [ -n "${SAHAI_SETUP_DNS_CREDENTIALS:-}" ]; then
    credentials_json="$(build_credentials_json_from_kv_list "$SAHAI_SETUP_DNS_CREDENTIALS")"
  elif [ "${SAHAI_SETUP_NONINTERACTIVE:-}" != "1" ]; then
    log "${dns_provider} が要求する認証情報を入力してください"
    log "例: cloudflareなら CF_DNS_API_TOKEN(一覧: https://go-acme.github.io/lego/dns/index.html)"
    log "キー名を空EnterでDNS設定を終了します。"
    local entries="[]" key value entry
    while true; do
      read -r -p "  環境変数名(空Enterで終了): " key
      [ -z "$key" ] && break
      read -r -s -p "  ${key} の値: " value; echo
      entry="$(jq -n --arg k "$key" --arg v "$value" '{key:$k, value:$v}')"
      entries="$(printf '%s' "$entries" | jq --argjson e "$entry" '. + [$e]')"
      unset value
    done
    credentials_json="$entries"
  fi

  local body
  body="$(jq -n --arg p "$dns_provider" --arg e "$acme_email" --argjson c "$credentials_json" \
    '{dns_provider:$p, acme_email:$e, credentials:$c}')"

  log "DNS/証明書設定を保存しています(最大1分ほどかかります)..."
  if ! api_put_body "/api/settings/dns-provider" "$body" -H "Authorization: Bearer $SAHAI_API_TOKEN_VALUE" --max-time 90 >/dev/null; then
    warn "DNS/証明書設定の保存に失敗しました。認証情報を確認してください。"
    warn "詳細: docker compose -f compose.yaml logs sahai-server traefik"
    return 1
  fi

  # dns_provider・acme_email・認証情報の保存先はDBと/var/sahai/.sahai.envであり
  # (sahai-serverが上記PUTの中で書く)、こちら側での控えは持たない
}

build_credentials_json_from_kv_list() {
  # "KEY1=VAL1,KEY2=VAL2" 形式(改行区切りも許容)をJSON配列へ変換する
  local input="$1"
  printf '%s' "$input" | tr ',' '\n' | jq -R 'select(length > 0) | split("=") | {key: .[0], value: (.[1:] | join("="))}' | jq -s '.'
}

# ============================================================
# 8. 証明書取得の確認
# ============================================================
step9_verify_certificate() {
  local domain="$1"
  if [ -z "$domain" ]; then
    warn "ドメインが未確定のため証明書確認をスキップします。"
    return 1
  fi
  if ! command -v openssl >/dev/null 2>&1; then
    log "opensslが無いため確認をスキップします。ブラウザで https://${domain} を開いてください。"
    return 0
  fi

  local attempt=0 max_attempts=4 issuer
  while [ "$attempt" -lt "$max_attempts" ]; do
    issuer="$(echo | openssl s_client -connect "${domain}:443" -servername "${domain}" 2>/dev/null \
      | openssl x509 -noout -issuer 2>/dev/null || true)"

    if [ -n "$issuer" ] && ! printf '%s' "$issuer" | grep -qi "TRAEFIK DEFAULT CERT"; then
      log "証明書のissuer: ${issuer#issuer=}"
      return 0
    fi

    attempt=$((attempt + 1))
    if [ "$attempt" -lt "$max_attempts" ]; then
      log "証明書の取得を待っています(${attempt}/${max_attempts})..."
      sleep 30
    fi
  done

  warn "Let's Encrypt証明書をまだ確認できません。以下を確認してください:"
  warn "以下を確認してください:"
  warn "  - DNS(${domain})がこのサーバーを指しているか(伝播に時間がかかる場合があります)"
  warn "  - APIトークンの権限(例: CloudflareならZone:DNS:Edit)"
  warn "  - docker compose -f compose.yaml logs traefik | grep -i acme"
  warn "  - 再確認: openssl s_client -connect ${domain}:443 -servername ${domain} </dev/null 2>/dev/null | openssl x509 -noout -issuer"
  return 1
}

# ============================================================
# 9. レジストリ資格情報のDB登録(sahai service create用)
# ============================================================
# step1で決定した値をここで登録する(APIトークン確定・サーバー起動後でなければ
# PUT /api/settings/registryを呼べないため)。step1で"provisioned"以外
# (reuse-existing/skip)だった場合は何もしない(reuse-existingは平文パスワードが
# 分からないため登録しようが無く、skipは意図的にスキップされている)。
step10_register_registry_credentials() {
  if [ "${SAHAI_REGISTRY_CREDENTIALS_MODE:-}" != "provisioned" ]; then
    return 0
  fi

  local body resp warning
  body="$(jq -n \
    --arg url "$SAHAI_REGISTRY_URL_VALUE" \
    --arg u "$SAHAI_REGISTRY_AUTH_USER_VALUE" \
    --arg p "$SAHAI_REGISTRY_AUTH_PASSWORD_VALUE" \
    '{registry_url:$url, registry_username:$u, registry_password:$p}')"
  resp="$(api_put_body "/api/settings/registry" "$body" -H "Authorization: Bearer $SAHAI_API_TOKEN_VALUE" || true)"

  warning="$(printf '%s' "$resp" | jq -r '.login_warning // empty' 2>/dev/null || true)"
  if [ -n "$warning" ]; then
    warn "警告: $warning"
    warn "設定は保存されています。Web UIの「レジストリ設定」から再確認できます。"
  else
    log "レジストリ資格情報を登録しました。"
  fi
}

# ============================================================
# 10. systemd導入確認
# ============================================================
step11_offer_systemd_install() {
  if ! command -v systemctl >/dev/null 2>&1; then
    log "systemctlが無いため、サービス登録をスキップしました。"
    return 0
  fi

  local do_install="${SAHAI_SETUP_INSTALL_SYSTEMD:-}"
  if [ -z "$do_install" ]; then
    if [ "${SAHAI_SETUP_NONINTERACTIVE:-}" = "1" ]; then
      do_install="false"
    else
      local ans
      read -r -p "systemdサービスとして登録しますか?(sudo権限が必要です) [y/N]: " ans
      if [ "$ans" = "y" ] || [ "$ans" = "Y" ]; then do_install="true"; else do_install="false"; fi
    fi
  fi

  [ "$do_install" = "true" ] || return 0

  if ! command -v sudo >/dev/null 2>&1 || ! sudo -n true 2>/dev/null; then
    warn "sudoを実行できませんでした。以下を手動で実行してください:"
    warn "  1. deploy/sahai.service の __SAHAI_REPO_DIR__ と __DOCKER_BIN__ を実際の値に置換"
    warn "  2. sudo install -m 644 deploy/sahai.service /etc/systemd/system/sahai.service"
    warn "  3. sudo systemctl daemon-reload && sudo systemctl enable sahai"
    return 0
  fi

  local docker_bin tmp_unit
  docker_bin="$(command -v docker)"
  tmp_unit="$(mktemp)"
  sed -e "s#__SAHAI_REPO_DIR__#${SCRIPT_DIR}#g" \
      -e "s#__DOCKER_BIN__#${docker_bin}#g" \
      "$SCRIPT_DIR/deploy/sahai.service" > "$tmp_unit"

  if sudo install -m 644 "$tmp_unit" /etc/systemd/system/sahai.service \
     && sudo systemctl daemon-reload \
     && sudo systemctl enable sahai; then
    log "systemdサービス sahai を登録しました(次回ブートから自動起動)。"
  else
    warn "systemdサービスの登録に失敗しました。deploy/sahai.service を参考に設定してください。"
  fi
  rm -f "$tmp_unit"
}

# ============================================================
# 11. 完了メッセージ
# ============================================================
step12_print_summary() {
  local domain="$1"
  log ""
  log "====================================================="
  log "sahai のセットアップが完了しました。"
  log ""
  log "  管理画面: https://sahai.${domain}"
  log "  APIトークン(この場だけの表示です。控えてください):"
  log "    ${SAHAI_API_TOKEN_VALUE}"
  log "  (このトークンは ${ENV_FILE} にも保存されています。パーミッション600。"
  log "   セットアップ再実行時の再利用にのみ使われます)"
  log ""
  if [ "${SAHAI_REGISTRY_CREDENTIALS_MODE:-}" = "provisioned" ]; then
    log "  レジストリ資格情報(この場だけの表示です。控えてください):"
    log "    URL:        ${SAHAI_REGISTRY_URL_VALUE:-registry.sahai.${domain}}"
    log "    ユーザー名: ${SAHAI_REGISTRY_AUTH_USER_VALUE}"
    log "    パスワード: ${SAHAI_REGISTRY_AUTH_PASSWORD_VALUE}"
    log "  (sahai service create用は設定済みです。ローカルからpushする場合のみ"
    log "   docker login ${SAHAI_REGISTRY_URL_VALUE:-registry.sahai.${domain}} を実行してください)"
  else
    log "  ローカルからpushする場合:"
    log "    docker login registry.sahai.${domain}"
  fi
  log ""
  unset SAHAI_REGISTRY_AUTH_PASSWORD_VALUE
  if [ "${LEGACY_ENV_FILE_MIGRATED:-}" = "1" ]; then
    log "  【要対応】APIトークンを ${LEGACY_ENV_FILE} から引き継ぎました。"
    log "  このファイルには古い認証情報が平文で残っています。"
    log "  現在は未使用のため、確認のうえ削除してください:"
    log "    rm ${LEGACY_ENV_FILE}"
    log ""
  fi
  if command -v systemctl >/dev/null 2>&1 && [ -f /etc/systemd/system/sahai.service ]; then
    log "  起動/停止:"
    log "    sudo systemctl start sahai"
    log "    sudo systemctl stop sahai"
    log ""
  fi
  log "====================================================="
}

main() {
  step0_check_prerequisites
  step1_configure_registry
  step2_ensure_api_token
  step3_write_setup_env
  step4_compose_up_build
  step5_wait_for_sahai_server_ready
  step6_run_initial_setup_if_needed
  step7_configure_dns_and_tls || true
  step9_verify_certificate "$SAHAI_DOMAIN_VALUE" || true
  step10_register_registry_credentials
  step11_offer_systemd_install
  step12_print_summary "$SAHAI_DOMAIN_VALUE"
}

main "$@"
