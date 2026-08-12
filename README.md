# 差配(Sahai)

Dockerホスト1台で動かす、小さなセルフホストPaaS。サービスを登録すると、サブドメインの割り当てとHTTPS化まで自動で行います。

単独の管理者が使うことを前提にした設計です(認証は固定のBearerトークン1本)。

## 主な特徴

- **image型 / compose型の両対応** — どちらの形式でも登録できます
- **サブドメイン自動割り当て** — `<サービス名>.<ベースドメイン>`
- **HTTPS自動化** — Let's Encrypt(DNS-01)。lego対応の100以上のDNSプロバイダから選べます
- **レジストリ同梱** — `registry:2` を同梱。CLIからそのままpushできます
- **死活監視** — 各コンテナのヘルスとCPU/メモリをWeb UIで確認できます

## 構成

```
Docker host (1台)
├─ sahai-server   API + Web UI + SQLite + Docker操作
├─ Traefik        ルーティング + Let's Encrypt
├─ registry       コンテナイメージレジストリ
└─ /var/sahai/    永続化データ
```

## 動作要件

- Docker Engine 20.10以降 + `docker compose` v2
- `jq`
- 取得済みのドメインと、このホストを指す2本のDNSレコード
  - `*.<ベースドメイン>` — 管理画面と各サービス
  - `*.sahai.<ベースドメイン>` — レジストリ(ワイルドカードは1階層しか一致しないため別途必要)
- DNSプロバイダのAPI認証情報([対応プロバイダ一覧](https://go-acme.github.io/lego/dns/index.html))

## セットアップ

```bash
git clone https://github.com/haroyan888/sahai.git
cd sahai
./setup.sh
```

`Permission denied` になる場合は `bash setup.sh` で実行できます。

Windowsは `.\setup.ps1` を実行してください。実行ポリシーで弾かれる場合は次のように起動します(スクリプトが恒久設定の変更を提案します)。

```powershell
pwsh -ExecutionPolicy Bypass -File .\setup.ps1
```

対話形式で「レジストリの資格情報 → ベースドメイン → DNS/証明書設定」を順に設定し、完了すると `https://sahai.<ベースドメイン>` にアクセスできます。表示されるAPIトークンは控えておいてください。

設定はすべてサーバー側のDBに保存され、リポジトリ内に設定ファイルは作られません。

サーバーではRustをビルドしません。公開済みイメージ(`haroyan/sahai-server`、amd64/arm64)を取得して起動します。取得できない場合(オフライン、未公開のフォーク等)はセットアップスクリプトがソースからのビルドに切り替えます。別のイメージを使うなら `SAHAI_IMAGE` で上書きできます。

```bash
SAHAI_IMAGE=myuser/sahai-server:v0.1.0 ./setup.sh
```

### 更新する

```bash
./update.sh     # Windowsは .\update.ps1
```

`git pull` してから土台の3コンテナ(traefik / sahai-server / registry)を新しいイメージで作り直します。設定・証明書・サービスのデータには触れません。DNS認証情報はsahai-serverが起動時に自動で再適用するため、更新後の操作は不要です。

**登録済みサービスは更新中も動き続けます。** これらはcomposeの管理外なので、土台の作り直しでは停止しません。一時的に止まるのは管理画面とレジストリだけです。

起動時にDBマイグレーションが走るため、**事前にDBのバックアップを取ります**(`/var/sahai/backups/` に5世代)。マイグレーションに失敗して起動しない場合は、バックアップを書き戻してください。

手元のソースのまま作り直すだけなら `--no-pull`(Windowsは `-NoPull`)を付けます。

### やり直す

```bash
./clean.sh      # Windowsは .\clean.ps1
```

コンテナ・DB・登録済みサービスをまとめて消し、セットアップ前の状態に戻します(確認プロンプトあり)。**設定ファイルだけを手で消すとDBが残って復旧できなくなる**ため、やり直すときはこのスクリプトを使ってください。

取得済みのTLS証明書は残します。Let's Encryptは同じ識別子の組に対して**7日間で5枚**までしか発行せず、やり直すたびに消すと数日間証明書を取れなくなるためです。証明書も消すなら `./clean.sh --acme`(Windowsは `-Acme`)を付けてください。

セットアップを何度も試すときは、Let's Encryptのstagingを使うと本番の枠を消費しません。証明書はブラウザに信頼されませんが、取得の確認はできます。

```bash
SAHAI_ACME_CA_SERVER=https://acme-staging-v02.api.letsencrypt.org/directory ./setup.sh
```

## CLI

バイナリ名は `sahai` です。タグ(`vX.Y.Z`)をpushすると[GitHub Actions](.github/workflows/release-cli.yml)がWindows・macOS(arm64)・Linux(x86_64/arm64)向けにビルドし、Releasesへ下書きとして公開します。同じタグで[サーバーのイメージ](.github/workflows/release-image.yml)もDocker Hubへ公開されます。

ソースからビルドする場合:

```bash
cargo build --release --bin sahai --manifest-path crates/sahai-cli/Cargo.toml
```

### 使い方

```bash
sahai login                        # APIトークンを保存
sahai service create myapp         # アップロード → サーバー側でビルド → 登録
sahai service list                 # 一覧
sahai service status myapp         # 状態・リソース使用量
sahai container push myapp --deploy  # ローカルでビルドしてpush、再デプロイ
```

ポート・環境変数・ボリュームの設定と初回の起動はWeb UIで行います。

## ドキュメント

| ファイル | 内容 |
|---|---|
| [development.md](docs/development.md) | 開発環境の起動と注意点 |
| [requirements.md](docs/requirements.md) | 要件定義 |
| [api-design.md](docs/api-design.md) | API設計 |
| [backend-architecture.md](docs/backend-architecture.md) | バックエンド構成 |
| [container-design.md](docs/container-design.md) | コンテナ構成 |
| [sequences.md](docs/sequences.md) | 主要フローのシーケンス |
| [test-strategy.md](docs/test-strategy.md) | テスト戦略 |
| [registry/README.md](registry/README.md) | レジストリ認証情報 |

## テスト

```bash
cargo test --workspace
cd web && npm test
```

ソースを編集しながら動かす場合の手順は[development.md](docs/development.md)を参照。

## ライセンス

本体は[MIT](LICENSE)です。配布物に含まれる第三者コードの著作権表示は次の2ファイルにまとめています。CLIのリリースアーカイブとサーバーイメージ(`/usr/share/doc/sahai/`)の両方に同梱されます。

| ファイル | 対象 |
|---|---|
| [THIRD-PARTY-LICENSES.html](THIRD-PARTY-LICENSES.html) | Rustクレート(CLI・サーバー共通、全対応プラットフォーム分) |
| [web/THIRD-PARTY-LICENSES.md](web/THIRD-PARTY-LICENSES.md) | Web UIのバンドルに入るnpmパッケージ |

依存を更新したら作り直してください。

```bash
cargo about generate about.hbs --workspace -o THIRD-PARTY-LICENSES.html
node scripts/gen-web-licenses.mjs
```

サーバーイメージのベースである`debian:bookworm-slim`とTraefik・`registry:2`は、それぞれの配布元のライセンスに従います。
