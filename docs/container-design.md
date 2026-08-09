# 差配(Sahai) コンテナ設計

差配(Sahai)自体を動かすためのコンテナ構成。要件定義書3章の構成図を、実際にデプロイ可能な形に落とし込む。Sahaiが**管理する側**の個々のサービス(ユーザーが登録したコンテナ)とは別の話である点に注意。

## 1. 全体構成

```
Docker host (1台)
├─ traefik           (compose.yaml管理下。80番〈HTTPSへのリダイレクト専用〉・443番を公開)
├─ sahai-server       (compose.yaml管理下。Dockerfileでビルド。Control Plane API+Web UI静的配信)
├─ registry           (compose.yaml管理下。registry:2)
└─ /var/sahai/         (ホスト上の永続化データルート。要件定義書3章。ディレクトリ自体は700)
    ├─ db/sahai.sqlite3         (600)
    ├─ .sahai.env               (DNSプロバイダ認証情報のブリッジファイル。600。4章参照)
    ├─ services/<id>/...        (sahai-serverが管理するサービスのボリューム)
    ├─ traefik/dynamic/         (sahai-serverが書き出す動的ルート定義。per-serviceの
    │                            ルートに加え、管理画面用のstatic-routes.ymlも起動時に
    │                            一度だけ書き出す。4章参照)
    ├─ traefik/acme/            (Let's Encrypt証明書。Traefikが管理)
    ├─ registry/                (レジストリのイメージストレージ)
    └─ compose-projects/<id>/   (compose型サービスのbase.yml/override.yml/.env置き場。sahai専用の.sahai.envとは無関係の、
                                 登録サービスごとの個別ファイル。.envは登録済みenv varsを平文で含むため600。要件定義書4章「秘匿値の保存」)
```

`traefik`/`sahai-server`/`registry` の3つは [compose.yaml](../compose.yaml) で一括管理する、いわば「差配を動かすための土台」。この3つ以外に、sahai-server自身が`docker run`/`docker compose`でユーザー登録済みサービスのコンテナを起動・停止する(要件定義書7章)。

Web UI(React SPA)とAPIはどちらも`sahai-server`が同一コンテナ内で配信する(単一ホスト運用でのコンテナ数・リソース削減のため。1.5章参照)。両者は当然同一オリジンになるため、Web UI側からのAPI呼び出しでCORSを意識せず開発できる(ただし別オリジン、例えばVite開発サーバーからの直接アクセスも許容するようCORSは緩めに設定済み。[api-design.md](./api-design.md) 1章参照)。

## 1.5 Web UIの配信(sahai-server統合)

Web UI(React SPA)は専用コンテナを持たず、`sahai-server`自身が`tower-http::ServeDir`でビルド済み静的ファイルを配信する。単一ホスト・単一デプロイ前提の差配では独立スケーリング・独立デプロイの必要が無く、コンテナ数を減らすほうが軽量・省リソースという方向性に合う。

[Dockerfile](../Dockerfile) は本番ビルドに関わる3ステージ(`web-builder`/`builder`/`runtime`)にこの統合の影響が及ぶ(開発用の`web-dev`/`dev`ステージは別途後述「開発時のホットループ」参照。ファイル全体では計5ステージ)。

- **web-builder**: `node:20-alpine`で`web/`を`npm run build`し、Viteの静的ビルド成果物(`web/dist/`)を生成するだけの専用ステージ
- **builder**: 従来通り`rust:1-slim-bookworm`で`sahai-server`をビルド
- **runtime**: `sahai-server`バイナリに加え、`web-builder`が生成した`web/dist`を`/app/web/dist`(`Config::web_dist_dir`の既定値)へコピー

react-router-domによるクライアントサイドルーティングのため、`/services`や`/not-service`のような深いパスへの直接アクセス・リロードが404にならないよう、axum側で`ServeDir::fallback(ServeFile::new(index.html))`によるSPAフォールバックを実装している(`api/mod.rs`参照)。**`ServeDir::not_found_service`はステータスを強制的に404にするため使えない**(SPAのクライアントサイドルーティングには200でindex.htmlを返す必要がある)。

Web UIはビルド時に`VITE_API_BASE_URL`を空文字のままにする(同一オリジン前提の相対パスfetchになる)。ただし[NotServicePage](../web/src/pages/NotServicePage.tsx)だけは、`sahai.example.com`以外の任意のサブドメイン(未登録サブドメイン・非HTTPサービスのサブドメイン)から表示されるため、`window.location.hostname`を明示的にクエリパラメータへ載せ、`https://sahai.example.com/api/not-service`へ**別オリジンで**問い合わせる(要件定義書6章参照)。

## 2. sahai-serverのDockerfile

[Dockerfile](../Dockerfile) はマルチステージビルド。

- **builder**: `rust:1-slim-bookworm`でworkspace全体をビルドし、`sahai-server`のリリースバイナリのみを取り出す
- **runtime**: `debian:bookworm-slim`をベースに、**Docker Engine本体(dockerd)は含めず**、Docker公式リポジトリから`docker-ce-cli`と`docker-compose-plugin`だけをインストールする

runtimeにdockerdを含めない理由: sahai-serverは`/var/run/docker.sock`をマウントしてホストのDocker Engineを直接操作する設計(要件定義書3章「技術選定」のbollard/docker composeの使い分け)であり、コンテナ内で別のDocker Engineを動かす(Docker-in-Docker)必要はない。`docker compose`サブプロセスを実行するにはクライアントCLIとcomposeプラグインさえあれば十分。

マイグレーション(`migrations/`)は`sqlx::migrate!`マクロによって**コンパイル時にバイナリへ埋め込まれる**ため、実行時イメージに`migrations/`ディレクトリをコピーする必要はない。

### イメージの配布

Rustのビルドは重く、サーバー(特にARMの小型機)でやらせると数十分かかる。そのためタグ(`vX.Y.Z`)のpushで
GitHub Actionsがamd64/arm64のイメージをビルドし、Docker Hub(`haroyan/sahai-server`)へ公開する
([release-image.yml](../.github/workflows/release-image.yml))。

アーキテクチャごとに**ネイティブランナー**(`ubuntu-latest`/`ubuntu-24.04-arm`)でビルドし、digestだけをpushしてから
マニフェストリストにまとめる。QEMUエミュレーションでのクロスビルドはRustだと極端に遅いため使わない。

`compose.yaml`は`image:`と`build:`の両方を持つ。実測した挙動は次の通り:

| 起動方法 | 挙動 |
|---|---|
| `up -d --pull always`(setupスクリプトが使う) | pullを試み、失敗したらローカルビルドへフォールバック |
| `up -d` | **pullせずローカルビルド**(イメージが手元に無い場合) |

つまり`--pull always`が無いとビルドされてしまうため、スクリプト側で必ず指定する。イメージが手元にあれば
どちらでも再ビルドは起きないので、systemd経由の`up -d`(起動のたび)でビルドが走ることはない。

## 3. `/var/sahai` のマウント一貫性(最重要の注意点)

要件定義書3章で最も重要な制約: **sahai-serverコンテナ自体も、ホストと同一パスで`/var/sahai`をマウントしなければならない**。

理由: sahai-serverはbollard/`docker compose`経由でホストのdockerdに対し `-v /var/sahai/services/<id>/...:...` のようなバインドマウント指示を送る。この時のパスは**ホスト側のパスとして dockerd に解釈される**(Docker-out-of-Dockerの典型的な罠)。sahai-serverコンテナ内から見えるパスとホストの実パスが食い違うと、sahai-server自身は正しいパスのつもりでも、実際にコンテナへマウントされる内容が変わってしまう。

[compose.yaml](../compose.yaml) では `sahai-server` サービスに `- /var/sahai:/var/sahai` を指定してこれを担保している。Traefikコンテナも同様に `/var/sahai/traefik/dynamic` をホストと同一パスでマウントする。

## 4. Traefikの静的設定

Traefikコンテナの`command`引数(`compose.yaml`/`dev.compose.yaml`のtraefikサービス参照)のポイント(要件定義書4章):

**静的設定はすべてCLIフラグとして渡し、`--configFile`は使わない**。Traefikは静的設定を単一のソースからのみ読み込み、`--configFile`使用時は他のCLIフラグ・環境変数からの静的設定が無視されるという既知の挙動があるため([traefik/traefik#7545](https://github.com/traefik/traefik/issues/7545))、`entryPoints`等と`certificatesResolvers`を分けて渡すとcertificatesResolversが無視されLet's Encrypt証明書取得ができなくなる。

- **DNS-01**。証明書取得自体は80番ポートに依存しない
- **80番(entryPoint `web`)はTraefikの静的設定としては素の待受のみを行う**。HTTP→HTTPSの恒久リダイレクトはentryPointの静的設定(`http.redirections.entryPoint`)では実装しない。entryPointの静的設定はTraefik起動後に変更できず、`SAHAI_HTTPS_REDIRECT`環境変数による起動時トグルができないため
- **HTTP→HTTPSリダイレクトは`RouteWriter::write_static_admin_routes`が生成する動的ルート側で行う**(`SAHAI_HTTPS_REDIRECT`環境変数、既定true。config.rs参照)。trueのとき、各ルーター(per-service・管理画面・registry・catchall)を`entryPoints: [websecure]`に限定し、`entryPoints: [web]`専用の`redirectScheme`ミドルウェアを持つ`https-redirect`ルーター(全パスにマッチ)を追加で書き出す。falseのときはこれらの制限・追加ルーターを一切書き出さず、各ルーターから`tls`ブロックも省略する。**`tls`キーの有無がプロトコル可用性を直接左右する**: `tls`を指定する〈空でも〉とそのルーターはwebsecure〈:443〉専用になりweb〈:80〉では一切応答せず、逆に`tls`を省略するとweb専用になりwebsecureでは一切応答しない。単一ルーターで両プロトコルに応答させることはできないため、falseのときは`tls`ブロック自体を省略してweb専用の平文httpルーターにする。この設計により、`SAHAI_DOMAIN`をローカルなドメイン(例: `localhost`)にしたテスト環境で実際のLet's Encrypt証明書を取得できず自己署名証明書のままになる場合でも、`SAHAI_HTTPS_REDIRECT=false`にすれば平文httpのまま証明書検証エラーなくアクセスできる。**ただし`registry`ルーターだけは`SAHAI_HTTPS_REDIRECT`の値に関わらず常に`tls`ブロック(`entryPoints: [websecure]`)を維持する**(`certResolver`はhttps_redirect=falseのとき省略し無駄なACME証明書取得を試みない)。`docker push`/`docker login`等のDockerツールチェーンは既定でHTTPS必須でplain httpへのフォールバックを行わないため、registryのtlsを消すと`docker push`が`404 Not Found`で失敗する(自己署名証明書でのHTTPS応答自体はDocker側が問題なく受け入れる)。**Web UI・API・CLIから利用する際は、`SAHAI_HTTPS_REDIRECT=false`の場合`http://sahai.<domain>`のように`http://`でアクセスする必要がある**(`https://`だと`tls`が省略されたルートに到達できず404になる。CLIの`config.toml`の`control_plane.url`も合わせて変更すること)
- **証明書解決(`certificatesResolvers`)**(resolver名は`letsencrypt`固定)。プロバイダ名は`SAHAI_DNS_PROVIDER`環境変数(既定値なし。特定プロバイダに固定しない)、通知先メールアドレスは`SAHAI_ACME_EMAIL`環境変数で指定する。Traefikが内部で使う[lego](https://github.com/go-acme/lego)ライブラリは100以上のDNSプロバイダに対応しており、`SAHAI_DNS_PROVIDER`とそのプロバイダが要求する認証情報の環境変数(下記)を差し替えるだけで別プロバイダに乗り換えられる。
  - プロバイダの認証情報(例: cloudflareなら`CF_DNS_API_TOKEN`)は`.sahai.env`(`SAHAI_DATA_ROOT`直下)に追加し、sahai-serverがbollard経由でTraefikコンテナ再作成時に直接Envとして渡す(compose.yamlのtraefikサービスに`env_file:`は無い。下記「DNS/証明書設定のWeb UI化」参照)。対応プロバイダと必要な環境変数の一覧: https://go-acme.github.io/lego/dns/index.html
- `file` providerで `/var/sahai/traefik/dynamic` を`watch: true`で監視する(`--providers.file.directory`/`--providers.file.watch`のCLIフラグで指定)。sahai-server側は登録時・名前変更時に加えstart/restart時にも毎回冪等にルートファイルを書き出すだけでよく(要件定義書6章「ポート割り当て」)、Traefikへ明示的なリロード指示を送る必要はない

### Traefikから Docker ホストの公開ポートへの到達性

`is_http`を持つサービスへのルーティング先は、sahai-serverが `http://<docker_host_address>:<host_port>` という形でTraefikルート定義に書き出す(`RouteWriter`、[backend-architecture.md](./backend-architecture.md) 3章)。TraefikコンテナからDockerホスト自身が公開しているポートへ到達させるため、`compose.yaml`のtraefikサービスに

```yaml
extra_hosts:
  - "host.docker.internal:host-gateway"
```

を設定し、`SAHAI_DOCKER_HOST_ADDRESS=host.docker.internal` をsahai-serverへ環境変数で渡している。これはDocker Engine 20.10以降のLinuxで有効な機能。

### 非HTTPサービス(Not Serviceページ)への到達性

`is_http`を持たないサービスの場合、sahai-serverはルート定義のbackendとして自分自身(`http://sahai-server:8080`、固定値。Web UI+APIを同一コンテナが配信するため、環境変数での上書きは不要)を書き出す。`traefik`と`sahai-server`は同一の`sahai`ブリッジネットワーク上にいるため、docker-composeが提供するサービス名ベースの名前解決(`sahai-server`)でTraefikからアクセスできる。

### 管理画面(`sahai.<domain>`)のパス分割ルートと、未登録サブドメイン用のワイルドカードcatch-all

管理画面自体のルート(不変)と未登録サブドメインの受け皿は、ユーザー登録済みサービスのルートと**同じ仕組み**(`RouteWriter::write_static_admin_routes`、[route_writer.rs](../crates/sahai-server/src/traefik/route_writer.rs))でsahai-server自身が起動時に一度だけ`/var/sahai/traefik/dynamic/static-routes.yml`へ書き出す。

このファイルをリポジトリ管理の静的YAMLとして`compose.yaml`から直接bind-mountする方式は使えない。`/var/sahai/traefik/dynamic:/var/sahai/traefik/dynamic:ro`で**ディレクトリ全体を読み取り専用マウント**している状態では、その内側に**単一ファイルを別途bind-mountする際のマウントポイント自体を作成できない**というDockerの制約があるためである(`docker run -v`でも同様に再現する)。

```
error mounting ".../static-routes.yml" to rootfs at "/var/sahai/traefik/dynamic/static-routes.yml":
create mountpoint for /var/sahai/traefik/dynamic/static-routes.yml mount: make mountpoint
"/var/sahai/traefik/dynamic/static-routes.yml": read-only file system
```

`:ro`を外せば回避はできるが、コンテナ終了後にホスト側へ空の`static-routes.yml`が残置される副作用があり、かつ「sahai-serverだけがこのディレクトリの書き手である」という一貫した設計方針からも外れる。そのため、bind-mountをやめてsahai-server自身がこの内容を生成する方式に統一している。

sahai-serverは`/var/sahai:/var/sahai`という自分専用の読み書き可能マウントを別途持っているため、Traefik側の`/var/sahai/traefik/dynamic:/var/sahai/traefik/dynamic:ro`(読み取り専用のまま)と衝突しない。

- `Host(sahai.<domain>)` → `sahai-server`(優先度100。転送先は`http://sahai-server:8080`固定。Web UIとAPIを同一コンテナが配信するため`/api`のパス分割は不要)
- `Host(registry.sahai.<domain>)` → `registry`(優先度100。転送先は環境変数`SAHAI_REGISTRY_INTERNAL_URL`、デフォルト`http://registry:5000`。要件定義書3章のレジストリをTraefik配下にホストする方針に対応。**このルートが無いとワイルドカードcatch-allに飲み込まれ、`docker login registry.sahai.<domain>`が誤ってWeb UIへ転送されてしまう**)
- `HostRegexp(^.+\.<domain>$)` → `sahai-server`(優先度1。個々のサービス用ルート・上記2つより低く設定し、どれにもマッチしなかった場合の受け皿とする。正規表現の`.`はドットにも一致するため`registry.sahai.<domain>`もここに掛かるが、優先度差でレジストリ用ルートが勝つ)

**静的ルートのルーター名・サービス名にはアンダースコアを使う**(`sahai_app`・`sahai_registry`・`sahai_catchall`)。Traefikのファイルプロバイダは`dynamic`ディレクトリ配下の全ファイルを1つの設定へマージするため、ルーター名は静的ルートと動的なサービス別ルートで同じ名前空間に載る。サービス別ルートのルーター名はサービス名そのままであり、サービス名は`[a-z0-9-]`しか使えないので、アンダースコアを含む名前なら利用者がどんなサービス名を付けても衝突しない。

ここでの`<domain>`は環境変数`SAHAI_DOMAIN`(既定値は無い。未設定の場合はセットアップ未完了の案内が出る。[config.rs](../crates/sahai-server/src/config.rs)参照)。sahai-server起動時に`Config::from_env`で読み込まれ、`RouteWriter`(上記のルート生成)とサービス登録時のsubdomain計算([naming.rs](../crates/sahai-core/src/naming.rs)の`subdomain_for`)の両方で共有される。

3つ目のワイルドカードルートは`*.<domain>`宛ての**ワイルドカード証明書**を要求するため、ルーターの`tls.domains`に`main: <domain>`・`sans: ["*.<domain>"]`を明示している(通常の個別サービス用ルートは`Host()`ルールから対象ドメインを自動導出できるためこの指定は不要)。

### 開発時のホットループ([dev.compose.yaml](../dev.compose.yaml))

本番はsahai-server1コンテナへ統合しているが、開発時はソース変更への反映速度を優先し、`dev.compose.yaml`という**本番用`compose.yaml`に一切依存しない、単体で完結したcomposeファイル**でtraefik・sahai-server・web・registryの4サービスを別途定義している。

```bash
cp dev.env.example dev.env   # 初回のみ。ポート等をデフォルトから変えたい場合に編集
docker compose -f dev.compose.yaml --env-file dev.env up -d --build
```

**公開ポートを持つのはtraefikのみ**(既定80。開発機で80番が別プロセスと衝突する場合は[dev.env.example](../dev.env.example)の`SAHAI_TRAEFIK_HTTP_PORT`で変更する)。sahai-server・web・registryはdockerネットワーク内部のみで完結し、すべてTraefik経由でアクセスする、本番と同じ「Traefikだけが外向きに出る」トポロジーを開発時も維持している。

- **sahai-server**: リポジトリ全体を`/app`へbindマウントし、リリースビルド済みバイナリではなく`cargo run -p sahai-server`で起動する(`Dockerfile`の`dev`ステージ、Rustツールチェーンのみでソースは含まない)。ソースを編集したら`docker compose ... restart sahai-server`で再起動すれば、cargoの増分コンパイル(実測5〜7秒程度)で変更が反映される
- **web**: `./web`を`/app/web`へbindマウントし、`npm run dev -- --host 0.0.0.0`(Viteの開発サーバー)で起動する(`Dockerfile`の`web-dev`ステージ)。ホスト側のファイル編集がそのままホットリロードされる
- **traefik・registry**: `compose.yaml`とほぼ同じ定義をこのファイル内に複製している(依存を断ち切るためのトレードオフとして許容した)。本番データ(`/var/sahai`)を汚さないよう、DB・レジストリ等のデータは別ディレクトリ(`/var/sahai-dev`)に分離している

**Web UI/APIのパス分割**: 本番ではWeb UI(静的ファイル)とAPIをsahai-server自身が単一のTraefikルートとして配信するが、開発時はsahai-server(cargo run)とweb(npm run dev)を別コンテナのまま起動するため、sahai-server自身が書き出すルート(常に自分自身への単一マージルート)の代わりに[traefik/dev-dynamic/dev-routes.yml](../traefik/dev-dynamic/dev-routes.yml)という開発専用の静的ルートでパス分割している(`/api/*` → sahai-server、それ以外 → web)。このファイルはTraefikコンテナ内の`/var/sahai/traefik/dynamic/`(sahai-serverが動的ルートを書き出すのと同じディレクトリ)へ、単一ファイルとして重ねてbind-mountしている。過去に踏んだ「read-onlyマウント済みディレクトリの中に単一ファイルを重ねてマウントできない」罠を避けるため、この開発用traefikのマウントだけは読み取り専用にしていない。

**dev環境のドメインはTraefikのGoテンプレート機能で動的に決まる**。`Host(sahai.localhost)`のような決め打ちにすると、LAN内の別端末から独自ドメイン(例: `sahai.desktop.example.com`)でアクセスしたい場合にルールがマッチせず、sahai-server自身が書き出すadmin routeへフォールバックしてしまい、dev用sahai-serverの持たない(古いビルドが残っていればそれが返る)静的ファイルが応答するという分かりにくい不具合になる。`dev-routes.yml`の`Host()`ルールは`Host(\`sahai.{{ env "SAHAI_DOMAIN" }}\`)`という形でTraefikコンテナ自身の環境変数`SAHAI_DOMAIN`(`dev.compose.yaml`のtraefikサービスに追加。既定`localhost`)を参照する。**この環境変数はコンテナ起動時に一度だけ読まれるため、Web UIのSettings画面で入力するドメインと[dev.env](../dev.env.example)の`SAHAI_DOMAIN`を必ず一致させること**(片方だけ変更すると一致しなくなり、このルールがマッチしなくなる)。priorityは200とし、sahai-server自身が書き出すadmin route(priority 100)を確実に上書きしつつ、Hostをsahai.<domain>限定にすることで登録済みサービス(`myapp.<domain>`等の動的per-serviceルート)とは競合しないようにしている。

**イメージ名の分離**: `sahai-server`/`web`とも本番用と別のイメージ名(`image: sahai-server-dev`/`sahai-web-dev`)を明示している。指定しないと`compose.yaml`と同じデフォルト名でビルドされ、開発用(`dev`/`web-dev`ステージ)のビルドが本番用(`runtime`ステージ)のイメージタグを上書きしてしまう。

**注意**: `compose.yaml`と`dev.compose.yaml`はどちらも(プロジェクト名を明示していないため)ディレクトリ名由来の同一プロジェクト名になり、コンテナ名・ネットワーク名が重複する。そのため本番と開発を同時には起動できず、切り替えるたびに一方を`down`してから他方を`up`する必要がある。

本番に戻すには`docker compose -f dev.compose.yaml down`してから`docker compose up -d --build`。

### DNS/証明書設定のWeb UI化(`.sahai.env`書き込み + Traefikコンテナ再作成)

`SAHAI_DNS_PROVIDER`・`SAHAI_ACME_EMAIL`・プロバイダ固有の認証情報(`CF_DNS_API_TOKEN`等)は、初回セットアップ後にWeb UIの「DNS/証明書設定」画面(`DnsConfigSection`、[api-design.md](./api-design.md) `GET/PUT /api/settings/dns-provider`)から変更できる。

これらの値はDBだけに移行することができない。Traefikの`certificatesResolvers`は静的設定(起動時のCLI引数、上記4章)としてしか渡せず、動的な`file` providerによるホットリロードの対象外だからである。そのためWeb UI側の保存操作は、DB更新に加えて**sahai-server自身が`.sahai.env`ファイルを書き換え、続けてTraefikコンテナを再作成する**、という他の設定項目にはない特別な処理になる。

`.sahai.env`(`SAHAI_DATA_ROOT`直下、既定`/var/sahai/.sahai.env`)は**sahai専用の内部ブリッジファイル**であり、利用者が直接編集するものではない。ファイルが存在しない場合は`env_file::upsert`が自動作成する。`SAHAI_DATA_ROOT`配下に置くのは、Windows(Docker Desktop)ではリポジトリのクローン先(`repo_dir`)をホスト同一パスでマウントできず、それ以外の場所だとTraefikコンテナ再作成時に読めなくなるためである(下記「Traefik再作成のbollard直接操作化」参照)。

**保存の処理順序**(`service::settings::update_dns_config`): ①入力バリデーション → ②`.sahai.env`書き込み(`env_file::upsert`、失敗時はここで中断しDB・メモリ上の状態は変更しない) → ③DB永続化 → ④メモリ上の設定(`AppState::settings`)更新 → ⑤Traefikコンテナ再作成。**Traefik再作成をあえて最後に置く**ことで、途中で失敗しても「DBとファイルは既に正しい状態」を保証できる(④より前で失敗すればファイル/DBとも未変更のまま、⑤で失敗しても手動で再作成を試みれば復旧できる)。

**Traefik再作成のbollard直接操作化**: `recreate_traefik`(container.rs)は`docker compose`をサブプロセスとして呼ぶ方式は使わない。それにはsahai-serverコンテナ内とホストDockerデーモンの両方から見て同一パスでcompose.yamlを読める必要があるが(`repo_dir`のホスト同一パスマウント)、Windows(Docker Desktop)ではコンテナ側マウント先にWindowsパス(例: `E:\repos\sahai`)を指定できず`mount path must be absolute`エラーになり成立しない。

代わりに、既存のTraefikコンテナ(ラベル`com.docker.compose.service=traefik`で検索)を`inspect`し、その設定(イメージ・起動コマンド・マウント・ポート・ネットワーク等)を複製して停止・削除・再作成する。Envだけは「イメージのデフォルトEnv + `.sahai.env`ファイルの最新内容」で組み立て直す(`env_file:`ディレクティブの実行時展開に相当)。これにより`repo_dir`・`SAHAI_REPO_DIR`・`SAHAI_COMPOSE_FILE`は完全に不要になり、Windows/Linux問わず同じ設計で動作する。`docker compose up`で最初に起動されるTraefik(compose.yamlのtraefikサービスに`env_file:`は無い)はDNS認証情報を持たない状態(自己署名証明書のまま)で、初期設定完了後にsahai-serverがbollardで認証情報入りに再作成する。

**ACME関連CLIフラグの差し替え**: 再作成時、起動コマンド(Cmd)は既存コンテナからそのまま複製するが、**ACMEの2つのフラグだけはDBの現在値で差し替える**:

- `--certificatesresolvers.letsencrypt.acme.email=<acme_email>`
- `--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=<dns_provider>`

この2つは`compose.yaml`が`${SAHAI_ACME_EMAIL:-}`・`${SAHAI_DNS_PROVIDER:-}`を**`docker compose up`の時点で**展開してCLIフラグへ埋め込むが、`docker compose`から見た値は**常に空**である(セットアップスクリプトはこれらをどこにも書き出さない)。実値はDBと`.sahai.env`が持ち、下記の差し替えとbollardによるEnv注入で反映する。この一元管理により、`docker compose up`が生成する設定が常に同一になり、再起動時にcomposeがTraefikコンテナを作り直して認証情報入りのEnvを失う事故を避けられる(compose.yamlのtraefikサービスに`env_file:`は無いため、作り直されると認証情報が消える)。Cmdを無条件に複製すると、その後DNS設定を何度保存し直してもこのフラグは空のままとなり、Traefikは`dnschallenge=true`でありながら`provider`が空という状態を「DNSチャレンジ未設定」と解釈し(HTTP・TLSチャレンジも未設定のため)、`cannot get ACME client ACME challenge not specified, please select TLS or HTTP or DNS Challenge`で証明書取得に失敗し続ける。証明書が取得できないとTraefikは組み込みの自己署名証明書を返すため、ブラウザなら警告を無視して管理画面へ到達できる一方、自己署名証明書を拒否しplain httpへフォールバックしないDockerツールチェーン(`docker login`/`docker push`)からはレジストリだけが到達不能になる、という分かりにくい症状になる。

差し替えは`override_acme_cmd_flags`(純粋関数、container.rs)で行い、上記2つのフラグに前方一致する要素のみ値を置き換え、他の引数には一切触れない。フラグ自体が存在しない場合(利用者が`compose.yaml`を独自に書き換えている場合等)は追加しない — 意図的にHTTPチャレンジ等へ変更している可能性があり、勝手にDNSチャレンジを注入すべきではないため。`dns_provider`が空の場合は`update_dns_config`のバリデーションが先に弾くため、空文字で上書きされることはない。

**フロントエンドの再接続UX**: Traefik再作成中はこの管理画面自体への接続も数秒切れる。ドメイン自体は変わらないため、`DnsConfigSection`は保存後に「再接続中」の状態を表示しつつ`GET /api/settings/dns-provider`を定期的に呼び直し、成功したら自動的に「再接続を確認しました」に切り替える(ポーリング、[DnsConfigSection.tsx](../web/src/components/DnsConfigSection.tsx))。保存リクエスト自体がTraefik再作成中の接続断で失敗して見えても(ネットワークエラー・タイムアウト等)、「バックエンド側の保存自体は成功している可能性がある」とみなしてこの再接続待ちに進む(明確なバリデーションエラー〈400〉のみ、再接続待ちに入らずその場でエラー表示する)。

**ドメイン変更との違い**: ドメイン(`SAHAI_DOMAIN`)自体を変更する場合はTraefikの動的ルートが即座に書き換わるため(コンテナ再作成不要)、旧ドメインのままのこの管理画面は自動的には再接続できない。SettingsPageはページ読み込み時点のドメインと保存後のドメインを比較し、変わっていれば「新しいURLへ移動してください」という案内を表示し、上記の自動再接続は試みない([SettingsPage.tsx](../web/src/pages/SettingsPage.tsx))。

**Docker Desktop特有の再作成タイミング問題**: このコンテナ再作成は「まさに今のリクエストを中継しているTraefik自身」を再作成するという特殊な状況のため、Windows/Docker Desktop環境(WSL2バックエンド)では、コンテナの`create`/`start`自体は正常終了するのに新しいコンテナがすぐには`running`状態にならないことがある。古いコンテナの停止・ネットワーク解放が完了するまで、観測ベースで数十秒〜1分程度かかることがある。`recreate_traefik`(container.rs)は`inspect_container`で実行状態(`state.running`)を確認しながら、起動していなければ`start_container`だけを軽量に再試行する(最大8回・6秒間隔)。

この事象はDocker Desktopの仮想化レイヤー(Hyper-V/WSL2)特有のネットワーク後始末の遅延が原因と推測しており、仮想化を挟まない本番のLinuxホストでは発生しない、または大幅に軽微になる可能性が高い。上記の再試行(最大48秒)で解消しない場合は、`docker start <container名>`を手動で実行すれば復旧できる(DB・`.sahai.env`は既に正しく保存されているため、この手動操作だけで問題ない)。

## 5. レジストリ

`registry:2` を素の状態で採用し、`REGISTRY_AUTH=htpasswd` による単独ユーザー認証とする(要件定義書3章)。認証情報の生成手順は [registry/README.md](../registry/README.md) に記載。ストレージは `/var/sahai/registry` に永続化する(4章「永続化データの配置」の一貫性のため、Sahai自身が管理するデータではないが同じ`/var/sahai`配下に置く)。

## 6. 初回セットアップ手順(概要)

**初期設定は`setup.sh`/`setup.ps1`が唯一の経路**である。`POST /api/setup`はサーバーが起動時に発行するセットアップトークンの提示を要求し、そのトークンはデータルート(700)配下にあるためコンテナ経由でしか読めない(要件定義書4章「セキュリティモデル」)。Web UIには初期設定画面を持たず、未設定の状態でアクセスするとスクリプトの実行を促す案内だけを表示する。

1. `setup.sh`(Linux/Mac)または`setup.ps1`(Windows)を実行する。スクリプトが以下を一括で行う
   - レジストリ資格情報の決定と`registry/auth/htpasswd`の生成(auto/manual選択式。registry/README.md参照)
   - `docker compose up -d --pull always`(公開済みイメージを取得して起動する)
   - セットアップトークンの取得と`POST /api/setup`による初期設定(ベースドメイン・APIトークン)
   - 任意でDNS/証明書設定(`PUT /api/settings/dns-provider`)とTraefikの再作成
2. スクリプトが表示するAPIトークンを控える。同じ値は`~/.config/sahai/setup.env`にも保存され、再実行時に再利用される
3. `docker login registry.sahai.<domain>`(以降のCLI pushで使用。要件定義書3章・12章)
4. ブラウザで`https://sahai.<domain>`を開き、控えたAPIトークンでログインする
5. Web UIで最初のサービスを登録し、`sahai container push` でビルド・デプロイ

DNSプロバイダ認証情報はスクリプトかWeb UIの「DNS/証明書設定」画面から設定する(上記4章参照。この2つが`.sahai.env`へ書き込む唯一の経路であり、利用者が直接編集する想定はない)。

## 7. 残課題

- **ワイルドカード証明書の実際の取得成功**は実機のDNSゾーンでの検証が済んでいない(ダミートークンでの構文検証のみ実施済み)
- Traefikの証明書ストレージ(`/var/sahai/traefik/acme`)のバックアップ方針は要件定義書2章の通りスコープ外
