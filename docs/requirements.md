# 差配(Sahai) 要件定義書

## 1. 概要・目的

Dockerホスト1台の上で動くサービス群を、Web UI/CLIから登録・起動・停止できる自作PaaS的システム「**差配(Sahai)**」。サブドメインとポートの割り当てを自動化し、Let's EncryptによるHTTPS化を行う。単独の管理者が運用することを前提とする(複数管理者・ロール分離は行わない。4章参照)。

- システム名「差配」は、「差配する(取り仕切る・指図する)」という動詞の意味と、江戸期に地主に代わって長屋の複数テナントを管理した代理人という語源に由来する。複数サービスを集約するだけでなく、能動的に管理・実行するという本システムの性格に合わせて選定した。
- CLIバイナリ名は `sahai` とする。
- ライセンスは **MIT** とする(リポジトリ直下の`LICENSE`参照)。依存する第三者コードは
  すべて許容的ライセンス(MIT・Apache-2.0・BSD・ISC・MPL-2.0等)に限り、コピーレフトの
  強いもの(GPL/AGPL/LGPL)は採用しない。著作権表示はCLIアーカイブとサーバーイメージの
  双方に同梱する。

## 2. スコープ

- 対象ホスト: Dockerホスト1台のみ(マルチホスト対応は行わない)
- 想定サービス規模: 数台〜十数台程度
- 利用者: 単独の管理者。複数ユーザー・ロール・権限分離は扱わない
- バックアップ: スコープ外(ディスク暗号化・バックアップはホスト側の責務)

## 3. システム構成

```
Docker host (1台)
├─ Control plane: API + DB + ポート割り当て + Docker操作 (Dockerコンテナとして稼働)
├─ Docker engine: 実際のコンテナ実行
├─ Traefik: サブドメインルーティング + Let's Encrypt(DNS-01)
└─ /var/sahai/ : 永続化データ置き場
```

- リバースプロキシは **Traefik** を採用。動的設定ディレクトリ(file provider)にルート定義を書き出すだけで反映されるため、Control plane側の実装が最小限で済む。
- コンテナイメージレジストリは **`registry:2`(Docker公式Distribution)** を採用し、`registry.sahai.example.com` としてTraefik配下にホストする。HarborのようなUI・RBAC・脆弱性スキャン機能は、単独ユーザー運用では過剰なため見送り、必要になった時点で移行を検討する。
- レジストリ認証は `REGISTRY_AUTH=htpasswd` を使用し、単独ユーザー1アカウント分の認証情報を初回セットアップ時に生成する。`sahai` 自体は認証情報を保持せず、事前に `docker login registry.sahai.example.com` 済みであることを前提とする。
- **永続化データの配置**: ホスト上のすべての永続化データを `/var/sahai/` 以下に集約する。
  ```
  /var/sahai/
  ├─ db/sahai.sqlite3            -- Control planeのSQLite DB
  ├─ services/<service_id>/<正規化パス>/  -- 各サービスの永続化ボリューム(6章参照)
  └─ traefik/dynamic/              -- Traefikの動的設定ファイル置き場(Control planeが書き出す)
  ```
- **Control plane自体のデプロイ形態**: Dockerコンテナとして稼働させる。ホストのDocker engineを操作するため `/var/run/docker.sock` をマウントする。加えて、Control planeはbollard/`docker compose`経由でdockerdに `-v /var/sahai/services/...` のようなバインドマウント指示を送るため、そのパスは**ホスト側のパスとして解釈される**。したがってControl planeコンテナ自体も `/var/sahai` を**ホストと同一パス**でマウントする(例: `-v /var/sahai:/var/sahai`)。Traefikコンテナも同様に `/var/sahai/traefik/dynamic` をホストと同一パスでマウントし、Control planeが書き出した設定を読めるようにする。

### 技術選定

- **Control plane**: Rust。Webフレームワークは `axum`、DBアクセスは `sqlx`(SQLite。async対応のため`rusqlite`より採用)。
  - Docker操作は `bollard`(Docker Engine APIラッパー)を、image型の `run`/`stop`/`inspect`/`stats`/`pull` に使用する
  - bollardはdocker-compose操作をサポートしないため、compose型の起動・停止は `docker compose` CLIをサブプロセスとして実行する
- **Web UI**: Vite + React。ビルド済み静的ファイルをControl plane自身が配信する(別コンテナは立てない)。

### API

REST API(`/api/*`)の全エンドポイント・リクエスト/レスポンス形式・エラーコードは [api-design.md](./api-design.md) を正とする。CLIの `container push` はビルド元マシンで直接 `docker build`/`docker push` するのみでAPIを経由しないため、APIのスコープは「登録済みサービスの管理」と「Control plane自身の設定」に限られる。ログ取得エンドポイントは設けない(9章参照)。

## 4. アクセス制御・ネットワーク

### ネットワークの前提

sahaiは**インターネットに公開しても、VPN等の閉じたネットワーク内に置いても動作する**。どちらで運用するかは利用者が決める(ファイアウォール・VPNの設定はsahaiの管理外)。公開する場合は下記「セキュリティモデル」を理解した上で運用すること。

| 項目 | 内容 |
|---|---|
| 管理画面 | Web UI・APIとも`sahai.<ベースドメイン>`固定(6章参照) |
| デプロイ済み各サービス | 各サービスのサブドメインで公開される。個別に公開/非公開を切り替える機能は持たない(必要ならホスト側のファイアウォールで制御する) |
| DNS管理 | 利用者が選択したDNSプロバイダに依存する(下記HTTPS参照)。sahai自体は特定プロバイダに固定していない |
| アウトバウンド通信 | 制限なし。compose型サービスが `build:` を持たない既製イメージ(mysql等)をDocker Hub等から直接pullすることを許容する |

### セキュリティモデル

単独管理者による運用を前提とし、認証は**固定のBearerトークン1本**で行う(OAuth・ユーザー管理は持たない)。トークンはセットアップ時に256bitの乱数として生成される。

- 通信はHTTPSを前提とする(下記HTTPS参照)。Bearerトークン方式でCookieを使わないため、CSRFの考慮は不要
- トークンの照合は**定数時間**で行い、先頭何文字まで一致したかが応答時間から推測されないようにする
- **初期設定前(トークン未設定)は、いかなるトークンでも認証を通さない**。未設定時のトークンは空文字列であり、素直に比較すると空のトークン(`Authorization: Bearer `)が一致してしまうため、空の期待値は常に不一致として扱う
- **初期設定(`POST /api/setup`)は、Bearerトークンの代わりにセットアップトークンの提示を要求する**。APIトークンがまだ存在しない段階で叩く必要があるため通常の認証層の外側に置かざるを得ないが、無防備にすると未設定の間に第三者が初期設定を先取りし、`api_token`を攻撃者の値で確定できてしまうため
  - サーバーは**未設定の状態で起動したときだけ**ワンタイムのトークンを生成し、`<データルート>/setup-token`へ600で書き出す。値はログに出さない
  - `setup.sh`/`setup.ps1`は`docker compose exec`でコンテナ内部からこのファイルを読み、`X-Sahai-Setup-Token`ヘッダーに載せて送る。手動で確認する場合は `docker compose -f compose.yaml exec -T sahai-server cat /var/sahai/setup-token`
  - ネットワーク越しの攻撃者はこのファイルを読めない。読めるのはDockerを操作できる利用者(=元々ホストを掌握できる立場)に限られる
  - 初期設定の成功時にトークンは失効する(ファイルを削除する)。設定済みの状態で起動した場合も、残っていれば削除する
- レート制限・アクセスログ・侵入検知は持たない(9章参照)。必要であればリバースプロキシ前段やホスト側で用意する
- 各サービスのコンテナはsahaiが権限分離せずに起動する。信頼できるイメージ・compose定義のみを登録すること

#### 秘匿値の保存

APIトークン・各サービスの環境変数・DNSプロバイダ認証情報はいずれも**平文で保存**し、ファイルのパーミッションをOSレベルの防御とする。対象と規則:

| パス | 権限 | 含む秘匿値 |
|---|---|---|
| `<データルート>` 直下のディレクトリ | 700 | (列挙防止。サービス名・ID・ボリューム構成) |
| `db/sahai.sqlite3` | 600 | APIトークン・レジストリ資格情報・全サービスのenv vars |
| `compose-projects/<service_id>/.env` | 600 | compose型サービスのenv varsを展開したもの |
| `.sahai.env` | 600 | DNSプロバイダの認証情報 |

いずれも**書き出しのたびに600を適用し直す**(緩いパーミッションで残っている既存ファイルを締め直すため)。

暗号化(sqlcipher等)は採用しない。Control planeは`restart: unless-stopped`で無人起動する常駐プロセスであり起動時に人手を介さず復号する必要があるため、復号鍵を同じディスク上に同じ600で置くことになり、ローカルのroot・dockerグループに対する防御力が増えないため(dockerグループは`docker run -v /:/host`で実質root相当)。鍵なしでファイルだけが外部へ出る場面(バックアップ・ディスク廃棄)にのみ効くが、そこはディスク暗号化(LUKS等)の担当領域であり、バックアップ自体が2章でスコープ外。

**Web UI側のトークン管理**: CLIは`sahai login`でトークンを`~/.config/sahai/config.toml`へ保存する(12章参照)が、ブラウザで動くWeb UIには同等のファイルシステムがないため専用のログイン画面(`/login`)を設ける。入力されたトークンは`localStorage`に保存し、以後のAPIリクエストの`Authorization`ヘッダーに使う。トークンが未保存の間は`/login`以外の全ルートへのアクセスを`/login`へリダイレクトする。ログアウト操作でトークンを削除して`/login`へ戻す。**未実装**: APIが`401`を返した場合(トークン変更等)に自動でログアウトさせる仕組みは無く、利用者は各画面のエラー表示から手動でログアウトする必要がある。

### ドメイン

`<ベースドメイン>`(`SAHAI_DOMAIN`環境変数)配下の`*.<ベースドメイン>`サブドメインを利用する。**サービスのsubdomainは常にこの配下に限定する**。

差配自身が使うホスト名は`sahai`配下にまとめる。利用者のサービスが並ぶ`*.<ベースドメイン>`と、差配の構成要素が並ぶ`*.sahai.<ベースドメイン>`を分けることで、どちらの持ち物かが名前だけで分かる。

| ホスト名 | 用途 |
|---|---|
| `sahai.<ベースドメイン>` | 管理画面(Web UI + API。同一ホストで`/`がUI、`/api/*`がAPI) |
| `registry.sahai.<ベースドメイン>` | コンテナイメージレジストリ |
| `<サービス名>.<ベースドメイン>` | 登録されたサービス |

- 必要なDNSレコードは`*.<ベースドメイン>`と`*.sahai.<ベースドメイン>`の2本。前者のワイルドカードは1ラベルしか一致しないため、後者を別に用意しないと`registry.sahai.<ベースドメイン>`が解決できない
- `sahai`のみ予約語としてサービス名に使用できない(`sahai-core::validation::RESERVED_SERVICE_NAMES`が完全一致のみ拒否)。`registry`は差配側が`sahai`配下へ移ったため予約しない
- **既定値は無い**(未設定のまま気付かずデプロイする事故を避けるため)。未設定の場合は初期設定が完了していない状態として扱う。`config.rs`のdocコメントが正とする
- 本ドキュメント中の`example.com`表記はすべて設定例であり、`SAHAI_DOMAIN`次第で変わる

### HTTPS

Let's Encryptの**DNS-01チャレンジ**を使用する(証明書取得自体に80番ポートは不要)。DNSプロバイダのAPI経由でTraefikが証明書を自動取得・更新する。

- 未登録サブドメインの受け皿ルート(6章参照)には`*.<ベースドメイン>`の**ワイルドカード証明書**を、管理画面・レジストリ・個々のサービスのサブドメインには`Host()`ルールから自動導出される個別証明書を使う。レジストリが2段のサブドメインでも、DNS-01で個別に取得するためワイルドカードの階層は問題にならない
- DNSプロバイダはTraefikが内部で使う[lego](https://github.com/go-acme/lego)が対応するもの(100以上)から`SAHAI_DNS_PROVIDER`で選択する。**既定値は無い**(特定プロバイダに固定しない方針)。未設定でも起動はでき、その場合Traefikは自己署名証明書のまま動作する。Web UIの「DNS/証明書設定」画面で選択・保存すると、その時点でTraefikコンテナが認証情報付きで再作成される
- Let's Encryptは**同じ識別子の組に対して7日間で5枚**という発行上限を持つ。取得済みの証明書は`SAHAI_DATA_ROOT/traefik/acme/acme.json`に保存され、`clean.sh`/`clean.ps1`は既定でこれを残す(消すと初期化のたびに1枚消費し、数日間再取得できなくなる)。繰り返し検証する場合は`SAHAI_ACME_CA_SERVER`でstagingへ向けられる
- **sahai-serverは起動時にTraefikコンテナの状態を点検し、DBと`.sahai.env`の現在値と食い違っていれば作り直す。** 認証情報はcompose.yamlではなくsahai-serverがbollard経由で渡す設計のため、`docker compose up`でTraefikが作り直されると認証情報が失われる。設定を保存し直すまで気付けないうえ、証明書の更新時になって初めて失敗するため、起動のたびに整合させる。一致していれば何もしない(毎回作り直すと不要な瞬断が起きる)
- 選んだプロバイダが要求する認証情報は、そのプロバイダが指定する環境変数名(例: Cloudflareなら`CF_DNS_API_TOKEN`)で`.sahai.env`(`SAHAI_DATA_ROOT`直下)に保存される。対応プロバイダと必要な環境変数の一覧: https://go-acme.github.io/lego/dns/index.html

### HTTP→HTTPSリダイレクト

`SAHAI_HTTPS_REDIRECT`環境変数(既定true)がtrueのとき、80番へのアクセスはすべて443番へ301恒久リダイレクトされる。証明書取得自体はDNS-01のため80番に依存しないが、httpで直接アクセスされた場合の利便性のために用意する。

falseにすると80番から平文httpのまま直接サービスへアクセスできる(443番は、tlsブロックを持たないルーターがTLSエントリーポイントに応答しなくなるため404になる)。**この場合、Web UI・CLIとも`http://`でアクセスする必要がある**(CLIの`config.toml`の`control_plane.url`も合わせて変更する)。`SAHAI_DOMAIN`を`localhost`等にしたテスト環境ではLet's Encrypt証明書を取得できず自己署名証明書のままになるため、falseにするとよい。

ただし**`registry`だけは`SAHAI_HTTPS_REDIRECT`の値に関わらず常にHTTPS(自己署名証明書)で応答する**。`docker push`/`docker login`等のDockerツールチェーンは既定でHTTPS必須でplain httpへフォールバックしないため。

リダイレクトはTraefikの静的entryPoint設定ではなく、sahai-server起動時に生成する動的ルート側で実現している(entryPointの静的設定は起動後に変更できないため。route_writer.rs参照)。

## 5. イメージ管理

- バージョン管理は行わず、**上書き方式**(常に最新イメージで置き換え)
- ビルド〜レジストリ登録はCLI(`sahai container push`)で行う。詳細は12章参照
- **Web UIでのサービス登録が先、CLIでのpushは後**という順序を必須とする。これにより「レジストリのイメージ名」と「Control planeのサービス名」の名前空間を一致させ、対応関係の曖昧さをなくす
- サービスのstart/restart時は、`docker run`/`docker compose up` の前に必ず `docker pull` を実行し、レジストリの最新イメージを取得してから起動する(ローカルキャッシュの古いイメージで起動されることを防ぐため)

## 6. サービス登録機能

登録時に以下を指定する:

- サービス名
- ソース種別: Dockerイメージ単体 or docker-compose
  - image型は暗黙的に1つのコンテナ(サービス名と同名の表示ラベルを持つ`ServiceContainer`)を持つものとして扱う
  - compose型は`compose_content`をパースし、含まれる各サービスをコンテナとして扱う。専用の「HTTPを喋るサービス名」フィールドは持たず、後述のポート指定で`is_http`を立てたコンテナが自動的にそれとなる。composeサービス名は12章と同じ規則(Dockerリポジトリ名として有効な文字のみ、合成タグ長128文字以内)でこの登録時点でも検証し、不正な場合は登録自体をエラーにする(CLI push時まで検知が遅れないようにするため)
- コンテナごとのポート(1つ以上。コンテナ内ポート・プロトコル(tcp/udp)・HTTPルーティング対象かどうかを指定。詳細は下記「ポート割り当て」参照)
- コンテナごとの永続化ボリューム(コンテナ内パスを指定。複数可)
- 環境変数(フォーム入力 または `.env` ファイルアップロード)

サブドメインは手動指定せず、**サービス名から自動生成**する(`<サービス名>.<SAHAI_DOMAIN>`。`SAHAI_DOMAIN`自体に既定値は無い。4章参照)。`name`にUNIQUE制約があるため、サブドメインも自動的に一意になる。

### ポート割り当て

- 1サービス・1コンテナにつき複数ポートを登録できる(HTTP以外の非HTTPポート、例えばDB直接接続用ポートなども登録可能)
- **`is_http`のポートはホストに公開しない**。サービスのコンテナは差配の`sahai`ネットワークに参加し、Traefikはコンテナ名で直接到達する(下記「HTTP公開ポートの到達経路」)。そのため`host_port`は指定しない
- **`is_http`以外のポートはホストに公開する**。DBへ直接つなぐ等、Traefikを介さない到達手段が目的のため。この場合の`host_port`は**手動指定**する(自動採番は行わない)
- ホスト側ポートに範囲の制限は設けない。ポート番号として有効な1〜65535であればよい
- 保存時に次の衝突を検証し、該当すればどのポートかを示すフィールド単位のエラーで拒否する。登録時・更新時の両方で同じ検証を行う。`is_http`のポートは`host_port`を持たないため検証の対象外
  - 他のサービスが既に使っているポート(全ServicePortを通してUNIQUE。DBのUNIQUE制約でも保証する)
  - 同一リクエスト内での重複
  - 差配自身がホストに公開しているポート(Traefikの80・443)。ここを奪うと差配全体が停止するため保存前に拒否する
- 差配の管理外で他のプロセスが使っているポートは検証しない。保存は通り、起動時にDockerがバインドに失敗する
- 各ポートには `protocol`(tcp/udp、デフォルトtcp)を指定する
- 複数ポートのうち、Traefikでルーティングする対象(HTTPを喋るポート)は**サービスにつき最大1つ**まで `is_http` フラグで指定できる
- サービス個別のTraefikルートは**start/restart時に毎回、その時点のDB状態から冪等に生成する**。`is_http`ポートがあればそのコンテナへ`http://svc-<ServiceContainer.id>:<container_port>`で向け、なければ下記「非HTTPサービスのルーティング」の案内ページへ向ける。これにより、ポート編集や`compose_content`編集による`is_http`対象の変更も、次のstart/restartで確実に反映される(名前変更時のみ、restartを待たず直ちに書き換える。下記「サービス名の命名規則」参照)
- **登録しただけでまだ起動していないサービスには個別ルートを作らない**。この場合はワイルドカードのcatch-allルート(下記「非HTTPサービスのルーティング」)が受け、Control planeがサブドメインからDBを引いて同じ案内ページを表示するため、個別ルートを前もって書き出す必要が無い

### HTTP公開ポートの到達経路

サービスのコンテナは、土台の3コンテナと同じ`sahai`ネットワーク(compose.yamlで`name: sahai`と固定)に参加する。Traefikはコンテナ名`svc-<ServiceContainer.id>`をDockerの組み込みDNSで解決し、コンテナ内ポートへ直接転送する。

**`is_http`のポートをホストに公開しないのはセキュリティ上の理由による。** 公開すると`https://<サービス名>.<ドメイン>`とは別に`http://<ホスト>:<host_port>`でも同じアプリに到達でき、そちらはTraefikを通らないためTLSもHTTP→HTTPSリダイレクトも効かない。公開サーバーでは意図しない平文の口になる。

副次的に、`is_http`ポートについてはホストポートの採番も衝突検証も不要になる。

`sahai`ネットワークの名前を`name:`で固定するのは、docker composeが既定でプロジェクト名(クローン先のディレクトリ名)を前置するため。固定しないとサービス側から`external`として参照できない。

### 非HTTPサービス・未登録サブドメインのルーティング(Not Serviceページ)

`is_http`ポートを持たないサービス(例: DB単体サービス)であっても、サブドメイン自体は自動生成され、Traefikルートも作成される。この場合のルーティング先は**Control plane自身(sahai-server)**とする。

Web UI(SPA)は`window.location.hostname`からアクセス元のサブドメインを判定し、`GET /api/not-service?host=<hostname>`(認証不要の公開API)へ問い合わせて「このサービスはHTTP/HTTPSを提供していません。登録されたポートへ直接接続してください」という案内ページを描画する。案内ページには、登録されている各ポート(ホストポート・コンテナポート・プロトコル)の一覧を表示する。

**未登録のサブドメイン**(そもそも登録されたことのないサービス名)へのアクセスも、この同じNot ServiceページのWeb UIフローに統一する。Traefikに`*.example.com`宛てのワイルドキャッチオールルート(個々のサービス用ルートより優先度を下げる)を静的に用意し、どのサービス用ルートにもマッチしなかったリクエストをすべてControl plane自身へ転送する。Web UI側は`/api/not-service`の応答が`found: false`の場合、「サービスが見つかりません」という案内を表示する(以前は、未登録サブドメインはTraefik自身の素の404だった)。

### サービス名の命名規則

サービス名は、Dockerイメージタグ・サブドメインラベル・composeプロジェクト名の一部として共用されるため、以下のルールでバリデーションする。

- 小文字英数字とハイフンのみ、先頭は英字、63文字以内(`^[a-z][a-z0-9-]{0,61}[a-z0-9]$` 相当)
- DB上 `name` にUNIQUE制約
- サービス名は登録後、**稼働中でも自由に変更可能**とする。実際のDockerコンテナ名・composeプロジェクト名は`svc-{id}`という不変の数値IDベースで管理しており(7章参照)、サービス名そのものには依存しないため、稼働中の名前変更もコンテナやcomposeプロジェクトの再作成なしに反映できる
- サブドメインはサービス名から自動生成されるため、名前変更時はサブドメインも連動して変わる。変更時は旧サブドメインのTraefikルート定義ファイルを削除し、新サブドメインのものを直ちに書き出し直す(稼働中でもこの切り替えは可能)
- image型は`ServiceContainer.name`(表示用ラベル)もサービス名と同一に保つ運用のため、名前変更時にあわせて更新する(表示上の整合性のためであり、実際のコンテナ識別には影響しない)
- この63文字制限は「`service-name` 単体がサブドメインラベルとして使われる」という制約に由来するものであり、compose型の合成タグ `<service-name>-<composeサービス名>` の長さ制限とは別物である。合成タグの妥当性検証は12章(container push)を参照

### compose_contentの編集

`compose_content`はPUTで変更可能とする(`source_type`自体は登録後固定であり、image型⇔compose型の変更は不可)。変更時の`ServiceContainer`同期ロジックは以下の通り:

1. 新しい`compose_content`をパースし、含まれるcomposeサービス名の集合を求める(このとき12章と同じ文字種・長さ検証を行う)
2. 既存の`ServiceContainer`一覧とdiffを取る:
   - **新規追加されたサービス**: 新しい`ServiceContainer`行を作成する(ポート・ボリュームは未設定の状態で作成されるため、同じ編集操作内で合わせて設定することを推奨する)
   - **削除されたサービス**: 対応する`ServiceContainer`行を削除する(`ServicePort`/`ServiceVolume`はCASCADEで削除)。当該コンテナが`is_http`ポートを持っていた場合、サービスは自動的に「非HTTPサービスのルーティング」の扱いに切り替わる。ボリュームパスは`service_id`のみに依存し`container_id`を含まないため(下記「永続化ボリューム規約」参照)、ホスト側のディレクトリ自体は自動削除されず残る
   - **既存のまま残るサービス**: `ServiceContainer.id`は変わらないため、Dockerコンテナ名(`svc-{id}`)は影響を受けない。ボリュームパスはそもそも`service_id`のみに依存するため、こちらも影響を受けない
3. 変更は保存のみで即時反映されず、次回restart時に反映される(他のメタデータ更新と同様。Traefikルートも再生成される)
4. restart時、Control planeは`docker compose up -d --remove-orphans`を使用し、新しい`compose_content`に存在しないサービスに対応する古いコンテナを確実に片付ける(7章参照)

なお、ボリュームパスが`service_id`のみに依存し`container_id`を含まないため、composeサービスキー名の変更(内部的には上記の「削除」+「新規追加」として扱われるケース)であっても、同じコンテナ内パスを指定している限り同じホストディレクトリを参照し続け、ボリュームデータは自然に引き継がれる。

### 永続化ボリューム規約

コンテナ内パスをもとに、ホスト側は以下の規約で自動マウントする(image型・compose型共通):

```
/var/sahai/services/<サービスID>/<コンテナ内パスを正規化した名前>/
```

例: `service_id=3` の `mysql` サービスでコンテナ内 `/var/lib/mysql` を指定 → ホスト側 `/var/sahai/services/3/var-lib-mysql/`

**サービス名ではなく数値ID(`service_id`)を使う。** これにより、サービス名を変更してもボリュームディレクトリのリネームが一切不要になる。

パスは`service_id`と正規化されたコンテナ内パスのみで決まり、`container_id`は含めない。そのため:
- 同一サービス内の複数コンテナが偶然同じコンテナ内パスを指定した場合、同じホストディレクトリを共有する(意図しない共有が起きうる)。これは通常のdocker-compose運用でもユーザー自身が管理すべき事項であり、sahai側では自動的な衝突回避は行わない
- compose_content編集(前述「compose_contentの編集」参照)でコンテナが実質的に再作成された場合(`ServiceContainer.id`が変わった場合)でも、同じコンテナ内パスを指定していれば同じホストディレクトリを参照し続けるため、ボリュームデータは自然に引き継がれる

サービス削除時のデフォルトは「残す」。`/var/sahai/services/<id>/` の削除は明示的な `purge_volumes` 指定時のみ行う(データ消失リスクを避けるため)。

## 7. 起動・停止・削除

- **起動(image型)**: `docker pull` → `docker run -d --name svc-{ServiceContainer.id} --restart unless-stopped -p {host_port}:{container_port}/{protocol} (ポート数分) -v ... --env-file ... {image}`
  - **pullはbollard経由のため、レジストリの資格情報をリクエストに明示的に添える。** bollardはDocker Engine APIを直接叩き、`docker login`が書く`~/.docker/config.json`を参照しない。添えないと匿名でのpullになり、htpasswd認証を要求する差配のレジストリからは取得できない(compose型は`docker compose pull`=CLI経由のため設定ファイルが効き、この問題は起きない)
  - 資格情報を添えるのは**イメージが差配のレジストリのものである場合のみ**。Docker Hub等の外部レジストリ宛のリクエストに差配の資格情報を送らない
- **compose_content中の`ports:`と`env_file:`は起動時に除去する**。どちらも差配が一元管理する項目であり、overrideでの上書きでは打ち消せないため、base側の記述自体を落とす。docker composeはこの2つを置き換えではなく**合算**するので、利用者の記述が残ったまま差配の設定が追加されてしまう(`image:`をoverrideで無効化できるのは、スカラーであり置き換えになるため)
  - `ports`: 利用者の公開設定が残ると意図しないポートがホストに開く。衝突検証はDBを見るだけなので、この経路で開いたポートはすり抜ける
  - `env_file`: 参照先は利用者のプロジェクト内にある相対パスであることが多いが、起動時のカレントは`compose-projects/<id>/`であり、そこにはbase.yml・override.yml・`.env`しか置かれない。存在しないファイルを指したまま起動しようとして失敗する。環境変数はWeb UIで設定したものだけを注入する
  - `volumes:`は除去しない。上の2つと違い**マウント先(target)単位でマージ**され、差配が管理するtargetは差配の設定が勝つ。利用者が別targetに足したボリュームはそのまま残る
- **起動(compose型)**: `docker compose pull`(overrideで`image:`を注入済みのため、build:の有無に関わらず全サービスが対象) → base composeに、登録済みの`ServiceContainer`ごとのポート・ボリューム・env varsを対応するcomposeサービスへ注入し、かつ全コンテナ(build:の有無に関わらず)に `container_name: svc-{ServiceContainer.id}` を注入したoverrideファイルを生成し `docker compose -f base.yml -f override.yml -p svc-{Service.id} up -d --remove-orphans`
  - 環境変数は**全コンテナに適用**する。登録されたenv varsからControl planeが `.env` ファイルを生成しプロジェクトルートに配置(変数展開用)、かつoverride生成時に全サービスへ `env_file` を注入する(実行時のコンテナへの反映用)。DBコンテナ等、HTTPを喋らないコンテナにも環境変数(例: `MYSQL_ROOT_PASSWORD`)が必要なケースに対応するため
  - `--remove-orphans` により、`compose_content` 編集で削除されたサービスに対応する古いコンテナを確実に片付ける
- **停止**: `docker stop svc-{ServiceContainer.id}`(image型)。compose型は `docker compose -p svc-{Service.id} down`
- **更新**: 停止 → 新イメージで起動(ダウンタイムあり、Blue-Green等は行わない)
- **起動失敗時のstatus**: `docker run`/`docker compose up`が失敗した場合、`status`を`error`に設定し、**失敗理由を`last_error`に保存する**。理由はDocker実行時の標準エラー出力であり、これが無いと利用者は`docker logs`まで降りないと原因が分からない。次回の起動が成功した時点でクリアする。表示が壊れないよう保存時に一定長で打ち切る。成功時は`running`、明示的な停止時は`stopped`とする。ヘルスチェックの結果によって`status`が変化することはない(`status`はライフサイクル操作の成否のみを表し、`health_status`とは完全に独立している)
- **コンテナ・プロジェクトの命名方針**: 実際のDockerコンテナ名は `svc-{ServiceContainer.id}`、composeプロジェクト名は `svc-{Service.id}` という不変の数値IDベースの識別子を使用する。これにより`Service.name`(サービス名)を変更しても、実行中のコンテナ・composeプロジェクトの実体には一切影響しない。Web UI/CLIでの**表示上**は `ServiceContainer.name`(image型はサービス名、compose型はcompose.yamlで定義されたサービス名)を用いる。内部識別子(`svc-{id}`)はユーザーに意識させない
- **Traefikルートの再生成**: start/restart時、Control planeはその時点の`is_http`ポート・`host_port`の状態からTraefikルート定義ファイルを冪等に再生成する(既に存在すれば上書きする)。登録時・名前変更時の即時書き出しと合わせて、ルートは常に最新のDB状態を反映する
- ボリュームの実ディレクトリ(`/var/sahai/services/<service_id>/...`)は**start時に存在しなければ作成する**(`mkdir -p` 相当)

### サービス削除フロー

削除は「外側(Traefikルート) → 中間(コンテナ) → 中心(DBレコード)」の順に、sahaiの中心に向かって行う。

1. Web UI: 削除確認ダイアログを表示
2. Traefikルート定義ファイルを削除する(実サービスへのルート・Not HTTP Serviceページへのルートいずれも、外部からの流入経路を最初に断つ)
3. コンテナ/composeプロジェクトを停止する(`docker stop svc-{container_id}` / `docker compose -p svc-{service_id} down`)
4. DBレコード(`Service`および関連する`ServiceContainer`/`ServicePort`/`ServiceVolume`もCASCADEで)を削除する。`purge_volumes=true`の場合は`/var/sahai/services/<id>/`も併せて削除する

`DELETE /api/services/{id_or_name}` はこの2〜4を内部で一括して行う。稼働中サービスに対しても直接呼び出し可能とし(「稼働中は拒否し、事前に`/stop`を呼ぶ」という制約は設けない)、Web UIは削除確認後に`DELETE`を呼ぶだけでよい。途中(ステップ2または3)で失敗した場合は処理を中断し、DBレコードは削除しない。ステップ3(コンテナ/composeプロジェクトの停止)は、**登録後一度もstartしていないサービス(対象のコンテナ/composeプロジェクトが元々存在しない)に対しても成功として扱う**(Dockerの「対象が見つからない」応答を「既に止まっている」と同義とみなす冪等な実装)。この考慮がないと、一度もstartしていないサービスがDELETEで永久に削除できなくなる。

### 排他制御

サービスの登録・更新・削除(特にポート割り当て)は、SQLiteの書き込みトランザクション(`BEGIN IMMEDIATE` 等)を用いて排他制御し、同時リクエストによる `host_port` 重複などの競合を防ぐ。

## 8. ヘルスチェック

- Control planeは`ServiceContainer`ごとにヘルス判定を行う。実際のDockerコンテナは `svc-{ServiceContainer.id}` という不変の名前で一意に特定できる(7章参照)ため、そのままコンテナ名で`docker inspect`すればよい(Docker Composeのラベル検索等は不要)
- 判定基準は以下の優先順位とする:
  1. 対象コンテナにDockerの `HEALTHCHECK` 命令が定義されている場合は、その判定結果(`healthy`/`unhealthy`/`starting`)を優先して使用する
  2. 定義されていない場合は、コンテナの実行状態(Running/Exited等)のみで判定する
- 判定結果は `ServiceContainer.health_status` / `last_health_check_at` に反映する。`Service.health_status` にはその中でのワーストケース集約値を反映する(一覧表示用)
- 異常時は管理画面上でステータス異常であることが分かる表示を行う(通知機能は不要)
- チェック間隔・失敗閾値: **10秒間隔、3回連続失敗で異常判定、1回成功で正常復帰**。この連続失敗カウントはDBに永続化せず、Control planeのバックグラウンドタスクのメモリ上で`ServiceContainer`単位に保持する(Control plane再起動時にリセットされるが、実害は復帰までの検知が最大30秒遅れる程度で許容する)
- サービスが停止中(status=stopped)の場合、UI上は健全性に関わらず一律「停止中」と表示する

**実行方式**: Control plane内でtokioのバックグラウンドタスクとして10秒おきに、running中の全サービスの全`ServiceContainer`へ判定を行い、結果をDBへ反映する。Web UIは数秒間隔でポーリングして表示する(WebSocket等は使わない)。

## 9. ログ

- システム側でログ収集・保存は行わない
- `docker logs` を直接確認する運用とする(MVPスコープ外)

## 10. リソース監視

- CPU/メモリ使用量の**監視のみ**実施(制限は設けない)
- Docker APIの統計情報(`docker stats` 相当)を管理画面でポーリング表示する想定

## 11. データモデル

```
Service
  id
  name                     -- UNIQUE制約。命名規則は6章参照。稼働中でも変更可
  subdomain                -- <name>.<SAHAI_DOMAIN> の形でサービス名から自動生成(NOT NULL、手動指定不可)。
                               SAHAI_DOMAINは環境変数のため、SQLiteのGENERATED列では表現できず、
                               通常列としてアプリケーション層(sahai_core::naming::subdomain_for)が
                               INSERT・name変更UPDATEの両方で明示的に計算・書き込む
  source_type              -- 'image' | 'compose'(登録後固定)
  image                    -- source_type=image の場合
  compose_content          -- source_type=compose の場合、ファイル本体を保存。PUTで変更可(6章参照)
  env_vars                 -- 環境変数(JSON object、平文保存。ファイルパーミッションで防御。下記4章「秘匿値の保存」参照)
  status                   -- stopped | running | error (起動失敗時にerror。7章参照)
  last_error               -- 起動失敗時のDocker標準エラー出力。起動成功でクリア(7章参照)
  health_status             -- unknown | healthy | unhealthy (ServiceContainerのワーストケース集約値。一覧表示用)
  last_health_check_at      -- ServiceContainerの中で最新のチェック時刻(一覧表示用)
  created_at / updated_at

ServiceContainer (Serviceに対し 1:N)
  id
  service_id
  name                      -- 表示用ラベル。image型はサービス名と同一、compose型はcompose.yamlのサービス名。
                               (service_id, name)でUNIQUE。実際のDockerコンテナ名(svc-{id})とは独立しており、
                               サービス名の変更やコンテナ実体には影響しない(7章参照)
  health_status              -- unknown | healthy | unhealthy (このコンテナ単体の判定結果)
  last_health_check_at

ServicePort (ServiceContainerに対し 1:N)
  id
  container_id               -- ServiceContainer.id
  container_port
  host_port                   -- 全ServicePortを通してUNIQUE。1-65535。is_httpのポートはホストに公開しないためNULL
  protocol                    -- tcp | udp (デフォルトtcp)
  is_http                     -- Traefikルーティング対象か(サービスにつき最大1件、アプリ層で担保)

ServiceVolume (ServiceContainerに対し 1:N)
  id
  container_id                -- ServiceContainer.id。このボリュームをどのコンテナに注入するか(起動時のマウント対象)を表す。
                                  ホスト側パスの決定には使われない(6章「永続化ボリューム規約」参照。service_idのみで決まる)
  container_path                -- コンテナ内マウントパス
```

`http_service_name`のような専用フィールドは持たない。compose型でHTTPを喋るコンテナは、`ServicePort.is_http=1`の行が属する`ServiceContainer`から導出する(2箇所で管理して不整合が生じることを避けるため)。

### Control plane自身の設定

サービスとは別軸の、Control plane自身の設定を保持する。単独管理者前提のため`settings`は常に1行のみ(`id`は固定値)。

```
Settings (1行のみ)
  id
  domain                     -- ベースドメイン。未設定(空文字列)ならセットアップ未完了(4章)
  https_redirect             -- HTTP→HTTPSリダイレクトの有無(4章)
  registry_url               -- 空なら domain から registry.sahai.<domain> を自動生成
  api_token                  -- APIのBearerトークン(4章「秘匿値の保存」)
  dns_provider / acme_email  -- DNS-01の設定。Traefikへは静的CLIフラグとして渡すため、
                                変更時はTraefikコンテナを再作成する(4章)
  registry_username          -- `sahai service create`のサーバー側push用。
  registry_password             利用者ローカルの`docker login`とは別の資格情報ストア
  updated_at

DnsProviderCredential (Settingsに対し 1:N)
  key                        -- プロバイダが要求する環境変数名(例: CF_DNS_API_TOKEN)
  value                      -- その値。同じ内容を`.sahai.env`にも書き出し、
                                Traefikコンテナ再作成時にEnvとして渡す
```

これらの値の正はDBであり、環境変数(`SAHAI_DOMAIN`等)はDBが空の初回起動時のみ読まれるシード用である。

## 12. CLIコマンド定義

### コマンド体系

```
sahai
├─ container push <name>            -- ビルド + レジストリpush(compose/image自動判別)
├─ service create <name>            -- プロジェクトをサーバーへアップロードし、サーバー側で
│                                       ビルド+push+サービスの新規登録まで一括で行う
├─ service update <name>            -- 登録済みサービスのプロジェクトを現在のディレクトリの
│                                       状態でサーバーへ再アップロードし、サーバー側でビルド
│                                       +pushを行う(上書き方式。`service create`の更新版)
├─ service list                     -- 登録済みサービス一覧(既定は人間可読なテーブル形式、
│                                       --jsonで生JSON出力)
├─ service status <name>            -- 詳細・ヘルス・リソース使用量を人間可読に整形表示
│                                       (--jsonで{"service":...,"stats":...}の生JSON出力)
├─ service start <name>             -- 起動結果の概要を整形表示(--jsonで生ServiceDetail)
├─ service stop <name>              -- 停止結果の概要を整形表示(--jsonで生ServiceDetail)
├─ service restart <name>           -- イメージ上書き後の再デプロイに使用。結果の概要を
│                                       整形表示(--jsonで生ServiceDetail)
├─ login                            -- Control plane APIへの認証トークンを保存
└─ config                           -- 設定ファイルのパス・設定可能項目・現在値の表示

GLOBAL OPTIONS(全サブコマンド共通):
  --insecure  TLS証明書検証をスキップする(既定false)。SAHAI_DOMAIN=localhost等、
              DNS-01でのLet's Encrypt証明書発行ができず自己署名証明書のままの
              ローカルテスト環境向け。config.tomlの`[control_plane]`に
              `insecure = true`を追記すれば毎回指定しなくてもよい(実運用のドメインでは使うべきではない)
```

コマンド体系は`container`/`service`の名前空間に整理している(後方互換のエイリアスは無く、旧`register`サブコマンド体系は存在しない)。`container push`は登録済みサービスの**コンテナイメージ**を更新する操作、`service create`はサービスの新規作成を表す。

CLIの責務は「ビルド+push」「サービスの追加(レコード作成)」「登録済みサービスへのライフサイクル操作の薄いラッパー」に限定する。**ポート・env・ボリュームなどのメタデータ設定、および起動操作は引き続きWeb UIの責務とする(`service create`で作成した直後のサービスもメタデータ未設定の状態であり、Web UIで設定してから起動する必要がある)。**

**存在確認失敗時のエラーメッセージ**: `GET /api/services/{name}`が失敗した場合、HTTP 404のときのみ「先にWeb UIで登録してください」と案内する。それ以外(ネットワーク到達不可・TLS証明書検証エラー・認証エラー等)は実際のエラー内容をそのまま表示する。以前は全エラーを一律「未登録」扱いにしていたため、実際には登録済みでも通信できないだけの状況が「未登録」と誤解される原因になっていた(自己署名証明書のドメインに対する実機検証で発覚)。

### `sahai container push <name>`

サービス名は事前にWeb UIで登録済みである必要がある。イメージ名前空間とサービス名前空間を一致させることで対応関係を一意にする。

```
OPTIONS:
  --context <path>       ビルドコンテキスト(default: .)
  --build-arg KEY=VALUE  ビルド引数(複数指定可)
  --platform <platform>  クロスビルド対象(例: linux/amd64、省略時はホスト依存)
  --deploy                push成功後、稼働中なら自動でrestartする(省略時はpushのみ)
```

処理フロー:

1. `GET /api/services/{name}` で存在確認(`{id_or_name}`にサービス名を渡す。api-design.md参照)
   - 存在しなければエラー終了(「先にWeb UIで登録してください」と案内)
   - 存在すれば登録済みの `source_type` を取得
2. `--context` 配下に compose ファイル(`docker-compose.yml/yaml`, `compose.yml/yaml`)があるか確認し、`source_type` と一致するか検証
   - 不一致ならエラー終了(登録内容とディレクトリ構成の食い違いを早期検知)
3. **compose ファイルがある場合(compose型)**:
   - compose を parse し、`build:` キーを持つサービスのみを抽出(既製イメージのサービスはビルド対象がないためスキップ)
   - 各サービスについて、タグ生成前に以下を検証する:
     - composeサービス名がDockerリポジトリ名として有効な文字(小文字英数字、`.`, `_`, `-`)のみで構成されているか(無効な文字を含む場合はエラー終了し、該当サービス名を明示)
     - 合成タグ `<service-name>-<composeサービス名>` の長さが128文字(Dockerのタグ長制限に合わせた安全な上限)を超えないか(超える場合はエラー終了)
   - 検証を通過したサービスについて `docker build -t registry.sahai.example.com/<service-name>-<composeサービス名>:latest -f <build.dockerfile> <build.context>` → `docker push`
4. **compose ファイルがない場合(image型)**:
   - `docker build -t registry.sahai.example.com/<service-name>:latest <context>` → `docker push`
5. `--deploy` 指定時のみ `POST /api/services/{name}/restart` を呼ぶ

レジストリへの認証は `sahai` 独自に持たせず、`docker login registry.sahai.example.com` を事前に済ませておく前提とする(認証情報の二重管理を避けるため)。なお、このイメージタグの命名(`<service-name>-<composeサービス名>`)はレジストリ上の名前空間の話であり、7章の実行時コンテナ名(`svc-{id}`)とは別の名前空間である。

### `sahai service create <name>`

`container push`が「Web UIで事前登録済みのサービス」への上書きpush専用なのに対し、`service create`は**未登録のプロジェクトをサーバーへアップロードし、サーバー側でビルド+push+サービスレコードの新規作成まで一括で行う**コマンド。ビルド自体をローカルではなくサーバー側で行うため、CLI側とControl plane側でタグ命名規則(`<service-name>-<composeサービス名>`)を独立に実装する必要がなくなる(サーバー側が両方を一箇所で行うため)。

```
OPTIONS:
  --context <path>       アップロードするプロジェクトディレクトリ(default: .)
  --build-arg KEY=VALUE  ビルド引数(複数指定可)
  --platform <platform>  クロスビルド対象(例: linux/amd64、省略時はホスト依存)
```

処理フロー:

1. `--context`配下を`.dockerignore`(無ければCLIの既定の除外ルール。`.git`等)を尊重してtar.gz化する
2. `POST /api/services/upload`へmultipart/form-data(`metadata`パート=JSON、`archive`パート=tar.gz)でアップロードする。CLIはビルド完了までHTTP接続をブロックして待ち、「登録中です。サーバー側でビルドしています...」という進捗表示を出す(同期処理。ジョブキュー/非同期ポーリングは行わない)
3. サーバー側は展開後のディレクトリ構成から image型/compose型を自動判定し(`container push`と同じ`find_compose_file`判定ロジックを共有)、`docker build`/`docker push`をサーバー自身の資格情報(Web UIの「レジストリ設定」カードから設定し、DBに保存する。`SAHAI_REGISTRY_USERNAME`/`SAHAI_REGISTRY_PASSWORD`環境変数は初回シード専用。registry/README.md参照)で実行する
4. ビルド成功後、サービスレコードを作成する。**ポート・env・ボリュームは一切指定しない**(空の状態で作成される)。作成されたサービスは、Web UIでメタデータを設定してから起動する必要がある
5. ビルドが1件でも失敗した場合はサービスレコードを作成しない(ビルド→登録の順で処理するため、失敗時は登録処理自体に到達しない)

`service create`で作成したサービスにレジストリへpushする資格情報はサーバー自身が保持する。この資格情報は`setup.sh`/`setup.ps1`のauto/manual選択メニューで初回起動時に自動設定される(詳細はregistry/README.md参照)ため、通常は利用者が意識する必要はない。Web UIの「レジストリ設定」カード(`/settings`画面、`GET/PUT /api/settings/registry`)は、パスワードローテーションや同梱の`registry:2`ではない外部レジストリへの切り替えを行いたい場合の**拡張設定**として引き続き利用できる。保存すると即座にサーバーが`docker login`を試みる(失敗しても設定自体は保存される)。`SAHAI_REGISTRY_USERNAME`/`SAHAI_REGISTRY_PASSWORD`環境変数はDBが空の初回起動時にのみ読まれる後方互換のシード用で(`setup.sh`/`setup.ps1`はこれらではなく`PUT /api/settings/registry`を直接呼ぶため使用しない)、以後はWeb UIで保存した値が正となる。これは`container push`が前提とする利用者のローカル`docker login`とは別の資格情報ストアであり、両者は独立に運用される(registry/README.md参照)。

### `sahai service update <name>`

`service create`は新規登録専用のため、登録済みサービスをサーバー側ビルドの仕組みで更新する手段がなかった(`container push`はローカル`docker build`が前提であり、サーバー側ビルドの資格情報とは別系統)。`service update`はこれを埋める、`service create`の更新版のコマンド。名前・`source_type`の変更はできず、ポート・env・ボリュームもここでは変更しない(引き続きWeb UIの責務)。

```
OPTIONS:
  --context <path>       アップロードするプロジェクトディレクトリ(default: .)
  --build-arg KEY=VALUE  ビルド引数(複数指定可)
  --platform <platform>  クロスビルド対象(例: linux/amd64、省略時はホスト依存)
  --deploy                push成功後、自動でrestartする(省略時はビルド+pushのみ)
```

処理フロー:

1. `--context`配下を`service create`と同じ規則でtar.gz化する
2. `POST /api/services/{name}/upload`へmultipart/form-data(`metadata`パート=JSON、`archive`パート=tar.gz)でアップロードする。`metadata`に`name`は含めない(対象サービスはパスの`{name}`で特定する)。`service create`と同様、ビルド完了までHTTP接続をブロックして待つ
3. サーバー側は対象サービスの存在確認(無ければ404)の後、アップロードされたプロジェクト構成(image型/compose型)が登録済みの`source_type`と一致するか検証する(不一致ならエラー終了)
4. **image型**: `docker build -t registry.sahai.example.com/<service-name>:latest <context>` → `docker push`(常に`:latest`のタグを上書きするため、DBの`image`列自体は変更不要)
5. **compose型**: `service create`と同じ規則で`build:`を持つ各サービスをビルド+push した後、新しい`compose_content`を保存し、6章「compose_contentの編集」と同じdiffロジックでサービス追加/削除を`ServiceContainer`へ反映する(既存コンテナのports/volumesは維持される)
6. `--deploy`指定時のみ`POST /api/services/{name}/restart`を呼ぶ(未指定の場合、ビルドしたイメージ・保存したcompose_contentの実際の反映には別途restartが必要。他のメタデータ更新と同様)

### compose型起動時のイメージ差し替え(Control plane側の実装事項)

Control planeに保存されている `compose_content` は、ユーザーが書いた元のcomposeファイル(`build:` キーが残ったまま)である。これをそのまま `docker compose up` すると、pushされたイメージではなくローカルで再ビルドしてしまう。

これを防ぐため、Control plane側の起動処理(7章のoverride生成ロジック)で、`compose_content` をパースして `build:` を持つ各サービスに対し `image: registry.sahai.example.com/<service-name>-<composeサービス名>:latest` をoverrideとして注入し、`build:` を無効化する。同じoverride生成の中で、登録済みの`ServiceContainer`ごとのポート・ボリューム・env vars・`container_name: svc-{ServiceContainer.id}`(build:の有無に関わらず全コンテナに対して)を対応するコンテナへ注入する(7章参照)。元のcomposeファイルは書き換えず、実行時にのみこれらの差し替えを行う。

**CLI側の命名規則(`<service-name>-<composeサービス名>`)とControl plane側のoverride生成ロジックは同じ規則を共有する前提とする。**

### `sahai service list / status / start / stop / restart`

Control planeのAPI(`GET /api/services`、`POST /api/services/{id}/start` 等)をそのまま叩く薄いラッパー。`<name>` はサービス名で解決する。`status`/`start`/`stop`/`restart`はいずれも既定で人間可読な整形表示を行い、`--json`指定時のみAPIレスポンスをそのまま`pretty print`する(`list`と同じ`--json`の慣習)。

`list`は既定で人間可読な固定幅テーブル(NAME/STATUS/HEALTH/TYPE/SUBDOMAIN列)を表示し、`--json`指定時は`GET /api/services`の生レスポンスをそのままpretty printする。

`status`は`GET /api/services/{name}`(`ServiceDetail`)と`GET /api/services/{name}/stats`(CPU/メモリ使用量)の2回のAPI呼び出しの結果を、既定では「名前・サブドメイン・種別・ステータス・ヘルス(最終チェック時刻)」のヘッダーと、コンテナ別の「NAME/HEALTH/CPU/MEM/PORTS/VOLUMES」テーブルにまとめて整形表示する(`route_warning`があれば併せて警告として表示)。`GET /api/services/{name}/health`は呼ばない — `HealthResponse`が返す`health_status`/`last_health_check_at`は`ServiceDetail`(サービス全体・コンテナ別とも)に既に含まれる完全な重複情報であり、2回の別呼び出しの間に状態が変わりうる不整合の余地もあるため。`--json`指定時は`{"service": ..., "stats": ...}`という1つのJSONオブジェクトを出力する(`health`キーは含めない)。

`start`/`stop`/`restart`は`ServiceDetail`から名前・`status`・`health_status`を抜き出した1行のメッセージ(例:「サービス 'myapp' を起動しました(status: running, health: unknown)。」)を既定表示とし、`route_warning`があれば続けて警告行を表示する。`--json`指定時は従来通り`ServiceDetail`をそのままpretty printする。

### `sahai login`

Control plane APIのBearerトークンを対話的に入力させ、`~/.config/sahai/config.toml` に保存する。

```toml
# ~/.config/sahai/config.toml
[control_plane]
url = "https://sahai.example.com"
token = "..."
# TLS証明書検証をスキップする(既定false)。--insecureの永続化に使う
insecure = false

[registry]
url = "registry.sahai.example.com"
```

### `sahai config`

設定ファイルの**パス**と、設定可能な全項目・その現在値を一覧表示する。**値を編集する機能は持たない**(変更は`sahai login`か、任意のエディタでこのファイルを直接編集する)。`token`は秘匿値のため伏せ字で表示する。

### エラーハンドリング方針

- ビルド失敗・push失敗はdocker CLIの終了コードをそのまま伝播し、標準エラーにも出力する
- Control plane API呼び出し失敗(認証切れ、ネットワーク不通等)は終了コード1で統一し、原因をわかりやすく表示する(トークン期限切れなら`sahai login`を促す等)
