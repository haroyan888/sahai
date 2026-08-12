# 開発環境ガイド

差配のソースを編集しながら動かすための手順と、踏みやすい落とし穴をまとめる。設計そのものは[container-design.md](./container-design.md)を参照。

本番用の`compose.yaml`とは別に、開発専用の[dev.compose.yaml](../dev.compose.yaml)がある。**本番用`compose.yaml`には一切依存しない、単体で完結したファイル**で、traefik・sahai-server・web・registryの4サービスを定義している。

## 起動する

```bash
cp dev.env.example dev.env
docker compose -f dev.compose.yaml --env-file dev.env up -d --build
```

`dev.env`が唯一の設定元になる。本番で`setup.sh`が対話で聞く項目(APIトークン・ドメイン・レジストリ資格情報など)は、すべてここから渡す。**開発では`setup.sh`を使わない**。

最低限、`SAHAI_API_TOKEN`と`SAHAI_DOMAIN`の両方を設定すると初期設定画面を通らずに起動できる。両方が非空でないとシードされない。

起動したら`http://sahai.localhost/`(既定のドメイン設定の場合)を開く。`dev.env`の`SAHAI_API_TOKEN`がそのままログイン用トークンになる。

止めるときは`down`。

```bash
docker compose -f dev.compose.yaml --env-file dev.env down
```

## ⚠️ プロジェクトルートから実行すること

`dev.compose.yaml`は`SAHAI_HOST_DATA_ROOT=${PWD}/data`という指定を持つ。この`${PWD}`は**composeファイルの位置ではなく、コマンドを実行したシェルのカレントディレクトリ**を指す。

別のディレクトリから`docker compose -f /path/to/sahai/dev.compose.yaml ...`と実行すると、サービスの永続ボリュームが意図しない場所(そのカレントディレクトリの`data/`)に作られる。**必ずリポジトリのルートで実行する。**

この変数が必要な理由は[container-design.md](./container-design.md)「データルートは2つの表現を持つ」を参照。要点だけ書くと、sahai-serverはコンテナの中から**ホストの**dockerdを操作するため、bindマウント元のパスは「コンテナ内から見えるパス」ではなく「dockerdから見たホスト側のパス」でなければならない。Windowsではこの2つを同じ文字列にできないので、別々に持っている。

## データの置き場所

生成されるデータはすべてプロジェクト直下の`./data`に出る(gitignore済み)。本番の`/var/sahai`と**同じ構造**にしてあるので、本番との差はルートのパスだけ。

```
data/
├── db/sahai.sqlite3        SQLite DB
├── uploads/                sahai service create のアップロード一時展開先
├── compose-projects/<id>/  compose型サービスのbase.yml・override.yml・.env
├── services/<id>/          サービスの永続ボリューム
├── registry/               レジストリのblob
├── registry-auth/htpasswd  レジストリの認証ファイル
├── traefik/dynamic/        sahai-serverが書き出すTraefikルート
├── traefik/acme/           取得したTLS証明書
└── .sahai.env              DNSプロバイダの認証情報
```

Windows開発機でも中身をそのまま覗けるようプロジェクト直下にしている(Docker Desktopの`/var`はVM内にあり、ホストからは実質見えない)。

**初期化したいときは`down`してから`./data`を消す。** DBを消すと`dev.env`の値が再度シードされる。`dev.env`を書き換えても反映されないのは、一度DBができると以降はDBの値が正になるため。

## 変更を反映する

| 変更した場所 | 反映方法 |
|---|---|
| Rust(sahai-server) | `docker compose -f dev.compose.yaml --env-file dev.env restart sahai-server`(コンテナ内で`cargo run`しているので増分コンパイルが走る。実測5〜30秒) |
| フロントエンド(`web/`) | Viteのホットリロードで即時。ただし**下記の例外**あり |
| `dev.compose.yaml`・`dev.env` | `up -d`で対象コンテナを作り直す |
| `traefik/dev-dynamic/dev-routes.yml` | Traefikがwatchしているので即時。ただし`{{ env }}`で参照している環境変数を変えた場合はtraefikの再作成が要る |

### ⚠️ Not Serviceページだけはホットリロードが効かない

`sahai.<domain>`宛てはViteが配信するが、**それ以外のサブドメイン(未登録サブドメイン・非HTTPサービス)はsahai-serverが配信する**。Traefikのcatch-allルートがsahai-serverへ送るためで、そこで返るのは`web/dist`(ホスト側で最後に`npm run build`した成果物)。

つまり`NotServicePage`まわりを変更したら、ビルドし直さないと反映されない。

```bash
cd web && npm run build
```

**ブラウザのスーパーリロード(Ctrl+F5)では解決しない。** 古いのはブラウザのキャッシュではなく、サーバーが返すファイルそのもの。

ページ単体の見た目を詰めるだけなら`http://sahai.<domain>/not-service`を直接開けばViteが配信するのでホットリロードが効く。振り分け込みで確認するときだけビルドが要る。

catch-allをViteへ向ければこの手間は無くなるが、そうすると開発環境が本番の経路(sahai-serverのSPAフォールバックによる`/not-service`への振り分け)を通らなくなり、回帰を踏めなくなる。意図的にこうしている([container-design.md](./container-design.md)「catch-allの転送先をsahai-serverから変えられない理由」)。

## アクセス経路

| URL | 配信元 |
|---|---|
| `sahai.<domain>/`(`/api`以外) | Vite(`web`コンテナ) |
| `sahai.<domain>/api/*` | sahai-server |
| `registry.sahai.<domain>` | registry |
| その他の`*.<domain>` | 起動中のサービス、無ければsahai-server(→`/not-service`へリダイレクト) |

**httpとhttpsで挙動は同じ**になる。`SAHAI_HTTPS_REDIRECT=true`ならhttpが301でhttpsへ寄せられ、falseなら両方が同じ内容を直接返す(websecure側は自己署名証明書)。片方だけ404になる場合はルート生成がおかしい。

外部公開ポートを持つのはtraefikだけで、他はDockerネットワーク内で完結する。本番と同じトポロジーを保つためで、sahai-serverやViteへ直接ポートを開けてはいけない。80番が別プロセスと衝突する場合は`dev.env`の`SAHAI_TRAEFIK_HTTP_PORT`を変える。

### ⚠️ ドメインは2箇所で一致させる

`dev.env`の`SAHAI_DOMAIN`はTraefikコンテナの環境変数として`dev-routes.yml`のGoテンプレートに埋め込まれ、**コンテナ起動時に一度だけ読まれる**。Web UIの設定画面で保存するドメインと食い違うとルールがマッチしなくなる。片方だけ変えないこと。変えたらcomposeごと再起動する。

## テスト

```bash
cargo test --workspace
```

```bash
cd web && npm test
```

Docker統合テストは実デーモンを要するため既定の`cargo test`から外してある。

```bash
cargo test -p sahai-server -- --ignored
```

**`--ignored`側は稼働中のsahai環境では実行しないこと。** 実際にコンテナを起動・削除し、Traefikコンテナの再作成まで行う。

手動確認の項目は[test-strategy.md](./test-strategy.md)のチェックリストにまとめてある。

## トラブルシューティング

**フロントの変更が反映されない** → Not Serviceページなら`npm run build`が要る(上記)。管理画面(`sahai.<domain>`)ならViteが配信しているので、`web`コンテナのログを見る。

**サービスへアクセスすると502** → Traefikは届いているがコンテナ側で失敗している。登録した`container_port`が、そのイメージが実際にlistenしているポートと一致しているか確認する。composeの`ports: "8080:80"`は**ホスト:コンテナ**なので、登録するのは右側の`80`。

Traefikコンテナから直接叩くと、Traefikが転送しようとしているのと同じ経路で確認できる。

```bash
docker exec sahai-traefik-1 wget -q -S -O /dev/null http://svc-<container_id>:<container_port>/
```

**未登録サブドメインでログイン画面が出る** → sahai-serverが古い。Rust側を変更したなら`restart sahai-server`。

**レジストリへのログインに失敗しましたと出る** → 起動順の問題で、sahai-serverがregistryより先に上がると初回だけ失敗する。`restart sahai-server`で解消する。

**生成されたルートを確認したい** → `data/traefik/dynamic/`を直接見る。sahai-serverが書き出したものと、リポジトリからマウントしている`dev-routes.yml`が並ぶ。

**サービスのボリュームの中身を見たい** → `data/services/<service_id>/<正規化したコンテナ内パス>/`にそのまま出ている(例: `/var/lib/mysql` → `var-lib-mysql`)。
