# 差配(Sahai) Control Plane(sahai-server)のマルチステージビルド

#
# Web UI(React SPA)は別コンテナ(nginx)ではなく、ビルド済み静的ファイルを
# sahai-server自身がtower-http::ServeDirで配信する形に統合している
# (2026-07-22、単一ホスト運用でのコンテナ数・リソース削減が目的。web/Dockerfileは廃止)。

# ---- web-builder(Web UIの静的ファイルをビルドするだけの専用ステージ) ----
FROM node:20-alpine AS web-builder
WORKDIR /app/web

COPY web/package.json web/package-lock.json ./
RUN npm ci

COPY web/ .
# バックエンドと同一オリジンで配信するため、APIベースURLはビルド時に空文字のまま
# (相対パスでの同一オリジンfetchになる。web/src/App.tsx参照)
RUN npm run build

# ---- web-dev(開発時のホットループ用。docker-compose.dev.yml参照) ----
# ソースはdocker-compose.dev.ymlがbindマウントするため、ここでは依存関係の
# インストールだけイメージにベイクしておく(コンテナ起動のたびにnpm ciし直さずに済む)
FROM node:20-alpine AS web-dev
WORKDIR /app/web

COPY web/package.json web/package-lock.json ./
RUN npm ci

CMD ["npm", "run", "dev", "--", "--host", "0.0.0.0"]

# ---- builder ----
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates/sahai-core/Cargo.toml crates/sahai-core/Cargo.toml
COPY crates/sahai-server/Cargo.toml crates/sahai-server/Cargo.toml
COPY crates/sahai-cli/Cargo.toml crates/sahai-cli/Cargo.toml
COPY . .

RUN cargo build --release -p sahai-server

# ---- dev(開発時のホットループ用。docker-compose.dev.yml参照) ----
# ソースは`docker-compose.dev.yml`がホストからbindマウントするため、ここではCOPYも
# `cargo build`もしない(起動コマンド`cargo run`が都度コンパイルする)。compose型
# サービスの起動/停止テストもできるよう、runtimeステージと同じDocker CLIを入れておく
FROM rust:1-slim-bookworm AS dev
WORKDIR /app

COPY docker/install-docker-cli.sh /tmp/install-docker-cli.sh
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates curl gnupg \
    && sh /tmp/install-docker-cli.sh \
    && rm /tmp/install-docker-cli.sh

# ---- runtime ----
FROM debian:bookworm-slim

# sahai-serverはcompose型サービスの起動/停止に`docker compose`をサブプロセスとして
# 呼び出す(bollardはdocker-composeを扱えないため)。
# そのためこのイメージにはDocker Engine本体(dockerd)は含めず、
# ホストのDocker socketに接続するクライアント(docker CLI + composeプラグイン)のみを入れる。
COPY docker/install-docker-cli.sh /tmp/install-docker-cli.sh
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg \
    && sh /tmp/install-docker-cli.sh \
    && rm /tmp/install-docker-cli.sh

COPY --from=builder /app/target/release/sahai-server /usr/local/bin/sahai-server
# migrationsディレクトリはsqlx::migrate!マクロによりビルド時にバイナリへ埋め込まれるため、
# 実行時イメージにコピーする必要はない

# Web UIの静的ファイル。Config::web_dist_dirの既定値(/app/web/dist)と一致させる
# (config.rs参照)
COPY --from=web-builder /app/web/dist /app/web/dist

# 配布物に含まれる第三者コード(Rustクレート・npmパッケージ)の著作権表示。
# MIT/Apache-2.0等は配布時に表示の複製を求めるため、イメージ自体に同梱する。
COPY LICENSE /usr/share/doc/sahai/LICENSE
COPY THIRD-PARTY-LICENSES.html /usr/share/doc/sahai/THIRD-PARTY-LICENSES.html
COPY web/THIRD-PARTY-LICENSES.md /usr/share/doc/sahai/THIRD-PARTY-LICENSES-web.md

WORKDIR /app
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/sahai-server"]
