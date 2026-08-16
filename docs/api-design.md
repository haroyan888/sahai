# 差配(Sahai) API詳細設計

[requirements.md](./requirements.md)の要件をAPIの形に落とし込む。JSONスキーマはTypeScript風の型注記で示す。

## 1. 共通事項

### 認証

`Authorization: Bearer <token>` が必須。欠如・不正の場合は `401` を返す(下記「エラーレスポンス形式」参照)。

### CORS

`/api/*`配下の全ルートにCORSを許可(`Access-Control-Allow-Origin: *`相当)している。本番ではWeb UIをsahai-server自身が同一オリジンで配信するためCORSは不要だが、開発時はVite開発サーバー(別オリジン)から直接APIを叩くため、オリジン制限をかけると開発が成立しない。

[NotServicePage](../web/src/pages/NotServicePage.tsx)は`sahai.<ベースドメイン>`以外の任意のサブドメインから表示されるが、これは別オリジンにはならない。catch-allルート・非HTTPサービス用ルートはいずれもパスを問わずsahai-serverへ転送するため、表示中のサブドメインのまま相対パスで`/api/not-service`を叩けば同じsahai-serverに届く([container-design.md](./container-design.md) 1.5章参照)。

認証はCookieの自動送信ではなくJSが明示的に付与するBearerトークンで行うため、オリジン制限はそもそもセキュリティ境界として機能していない(ブラウザ以外のクライアントはCORSに拘束されない)。よってCORSを緩めること自体による追加のリスクは無い(要件定義書4章「セキュリティモデル」)。

### 日時形式

すべて `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` 形式(ミリ秒付きUTC ISO8601、例: `2026-07-18T12:00:00.000Z`)。DBの型と1:1で対応させる。

### `{id_or_name}` の解決

数値のみで構成される文字列は`id`として、それ以外は`name`として解決する(`name`の命名規則上、先頭は英字必須のため数値のみの`name`は存在せず曖昧さはない)。該当なしは`404`。

### エラーレスポンス形式

```ts
{
  error: {
    code: string,       // 下記「エラーコード一覧」参照
    message: string,    // 人間可読なサマリ
    fields?: {           // バリデーションエラー時のみ。複数エラーは一度にまとめて返す(fail-fastにしない)
      field: string,     // 例: "name", "containers[0].ports[1].host_port"
      message: string
    }[]
  }
}
```

### エラーコード一覧とHTTPステータス

| code | HTTP | 用途 |
|---|---|---|
| `UNAUTHORIZED` | 401 | Bearerトークン欠如・不正 |
| `NOT_FOUND` | 404 | `{id_or_name}`に該当するサービスなし |
| `VALIDATION_ERROR` | 400 | リクエストボディの形式・値が不正(`fields`を伴う) |
| `CONFLICT` | 409 | `name`のUNIQUE制約違反、その他一意性の衝突(`host_port`は保存前に検証して`400`で返すため、競合時のみここに落ちる) |
| `UNPROCESSABLE` | 422 | 形式は正しいが意味的に矛盾(例: `source_type=image`なのに`compose_content`を指定) |
| `BUILD_FAILED` | 422 | `POST /api/services/upload`・`POST /api/services/{id_or_name}/upload`でのサーバー側`docker build`/`docker push`失敗(`docker`コマンドのstderrを含む) |
| `INTERNAL_ERROR` | 500 | 予期しない失敗(DB接続断等) |

**注**: Docker操作(`docker run`/`docker compose up`等)自体の失敗は、API呼び出しとしては成功とみなし`200`を返す。結果は返却される`Service`表現の`status: "error"`で表現する(7章「起動失敗時のstatus」)。API層のエラーとDocker操作結果のエラーを分離するのが目的。

## 2. リソース表現

### Service

```ts
type Service = {
  id: number,
  name: string,
  subdomain: string,                          // "<name>.<SAHAI_DOMAIN>"(例: "<name>.example.com")。読み取り専用
  source_type: "image" | "compose",           // 登録後は変更不可
  image: string | null,                        // source_type=image のときのみ非null
  compose_content: string | null,               // source_type=compose のときのみ非null
  env_vars: Record<string, string>,
  status: "stopped" | "running" | "error",
  last_error: string | null,                    // 起動失敗時のDocker標準エラー出力。起動成功でnullに戻る
  health_status: "unknown" | "healthy" | "unhealthy",
  last_health_check_at: string | null,
  created_at: string,
  updated_at: string,
}
```

一覧(`GET /api/services`)ではこの形のみ。詳細(`GET /api/services/{id_or_name}`)では下記`containers`を追加したものを返す。

### ServiceDetail (= Service + containers)

```ts
type ServiceDetail = Service & {
  containers: ServiceContainer[]
  route_warning?: string,   // start直後のみ現れうる。下記「POST .../start」参照
}

type ServiceContainer = {
  id: number,
  name: string,                                 // image型はService.nameと同一、compose型はcompose.yamlのサービス名
  health_status: "unknown" | "healthy" | "unhealthy",
  last_health_check_at: string | null,
  ports: ServicePort[],
  volumes: ServiceVolume[],
}

type ServicePort = {
  id: number,
  container_port: number,
  host_port: number | null,                      // is_httpのポートはホストに公開しないためnull
  protocol: "tcp" | "udp",
  is_http: boolean,
}

type ServiceVolume = {
  id: number,
  container_path: string,
}
```

### Settings

Control plane自身の基本設定。`GET/PUT /api/settings`・`POST /api/setup`で使う。`registry_url`/`registry_username`/`registry_password`は含めない(専用の`RegistryConfig`/「レジストリ設定」画面でのみ変更できる)。

```ts
type Settings = {
  domain: string,
  https_redirect: boolean,
  api_token: string,
}
```

### DnsConfig

DNSプロバイダ・ACME証明書関連の設定。`GET/PUT /api/settings/dns-provider`で使う。

```ts
type DnsConfig = {
  dns_provider: string,          // legoが対応するプロバイダ名(例: "cloudflare")
  acme_email: string,
  credentials: { key: string, value: string }[],  // プロバイダが要求する環境変数名と値
                                                     // (例: { key: "CF_DNS_API_TOKEN", value: "..." })
}
```

### RegistryConfig

`sahai service create`(サーバー側build+push)が使うレジストリの接続先・資格情報。`GET/PUT /api/settings/registry`で使う。

```ts
type RegistryConfig = {
  registry_url: string,             // 空欄で送るとdomainから`registry.sahai.<domain>`を自動生成する
  registry_username: string | null, // usernameとpasswordは両方nullか両方非nullのどちらかのみ許可
  registry_password: string | null, // GETでも平文のまま返す(DnsConfig.credentialsと同じ方針)
  login_warning?: string,           // PUTがdocker loginに失敗した場合のみ含まれる。
                                     // 保存自体は成功しているため200で返る(下記エンドポイント詳細参照)
}
```

## 3. エンドポイント詳細

### `POST /api/services` — 登録

**リクエストボディ**

```ts
type CreateServiceRequest = {
  name: string,
  source_type: "image" | "compose",
  image?: string,                               // source_type=image のとき必須
  compose_content?: string,                     // source_type=compose のとき必須
  env_vars?: Record<string, string>,            // 省略時は {}
  containers: {
    name: string,                                // 下記「containers[].nameの検証」参照
    ports?: {
      container_port: number,
      host_port: number | null,          // is_httpのポートはホストに公開しないためnull
      protocol?: "tcp" | "udp",                  // 省略時は "tcp"
      is_http?: boolean,                          // 省略時は false
    }[],
    volumes?: {
      container_path: string,
    }[],
  }[],
}
```

**`containers[].name` の検証**:
- `source_type=image` の場合: `containers`は要素1件のみ許可。`name`は`Service.name`と**完全一致必須**(不一致は`VALIDATION_ERROR`)。冗長に見えるが、POST/PUTのボディ形状をimage型・compose型で統一するためこの形にする(要件定義書6章の「image型もcompose型と同じ構造で扱う」方針を踏襲)
- `source_type=compose` の場合: `containers[].name`に含まれる値は、`compose_content`をパースして得られるcomposeサービス名の**部分集合**でなければならない(それ以外の名前が含まれていたら`VALIDATION_ERROR`)。逆に、パースして得られたサービス名のうち`containers`に登場しないものは、ports/volumesが空の`ServiceContainer`として作成される(要件定義書6章「compose_contentの編集」の「新規追加されたサービス」と同じ扱い。明示は任意)

**処理**: サービス名バリデーション → (compose型なら)composeサービス名バリデーション(6章・12章共通ロジック) → `containers[].name`の整合性チェック → `host_port`の検証(有効値・予約ポート・リクエスト内重複・既存サービスとの重複。`is_http`のポートは対象外) → `is_http`が全コンテナを通してサービスにつき最大1件であることのチェック(マイグレーションのコメント通りDBでは強制されないためアプリ層で検証。違反時は`VALIDATION_ERROR`) → DB挿入(`BEGIN IMMEDIATE`トランザクション、7章「排他制御」) → Traefikルート書き出し。

**レスポンス**: `201 Created`、ボディは`ServiceDetail`。

**エラー**:
- `name`重複 → `409 CONFLICT`
- バリデーション違反(パターン不一致、予約語〈`sahai`。要件定義書4章、下記参照〉、`containers[].name`に実在しない名前が含まれる、`is_http`重複、`host_port`の衝突等) → `400 VALIDATION_ERROR`

`host_port`の衝突を`409`ではなく`400`にしているのは、利用者が値を直せば解決する点で他のバリデーションと同じであり、`field`(`containers[0].ports[1].host_port`)を返して該当の入力欄に紐付けたいため。DBのUNIQUE制約は残しており、検証と挿入の間に別リクエストが同じポートを取る競合時のみ`409 CONFLICT`になる。
- `source_type=image`なのに`compose_content`を指定 等の相互排他違反 → `422 UNPROCESSABLE`

**予約語チェック**: `sahai`(管理画面)は静的Traefikルートのサブドメインと衝突するため、`name`として使用できない。レジストリは`registry.sahai.<domain>`にありサービスの`<name>.<domain>`とは階層が違うため、`registry`は予約しない(完全一致のみ拒否。例: `sahai-app`は許可される)。`crates/sahai-core/src/validation.rs`の`RESERVED_SERVICE_NAMES`参照。PUTでの名前変更時も同じチェックが働く。

### `POST /api/services/upload` — アップロードによる新規登録(`sahai service create`専用)

`multipart/form-data`。上記`POST /api/services`とは異なり、`source_type`/`image`/`compose_content`/`containers`はクライアントから指定しない。metadataの`compose_file`で使用するcomposeファイルを明示できる(省略時は既定の4つの名前から自動探索する)。プロジェクトのソースコードそのもの(tar.gz)を送り、サーバー側でビルド+push+登録まで行う(要件定義書12章)。

**リクエストパート**:

| パート名 | Content-Type | 内容 |
|---|---|---|
| `metadata` | `application/json` | `{ name: string, build_args?: { key: string, value: string }[], platform?: string }` |
| `archive` | `application/gzip` | プロジェクトディレクトリのtar.gz |

**処理**: `name`バリデーション・重複チェック・レジストリ資格情報(Web UIの「レジストリ設定」カードから設定。DBの`registry_username`/`registry_password`)の設定チェック → tar.gz展開 → `find_compose_file`によるimage/compose自動判定 → (compose型なら全サービスのタグ名を先に検証してから)`docker build`/`docker push` → `POST /api/services`と同じ`registration::create`ロジックでDB登録。`containers`は常に空(image型は`{name: Service.name}`相当の1件を自動補完、compose型は`compose_content`から検出した全サービスがports/volumes空で自動作成される。上記「`containers[].name`の検証」参照)。

**このエンドポイントはビルド完了まで同期的にブロックする**(ジョブキュー/非同期ポーリングは無い)。ビルドに数分かかりうるため、このルートのみ`axum::extract::DefaultBodyLimit`を既定の2MBから500MBへ引き上げている。

**レスポンス**: `201 Created`、ボディは`ServiceDetail`(`POST /api/services`と同形式)。

**エラー**:
- `name`重複 → `409 CONFLICT`
- `name`バリデーション違反、レジストリ資格情報未設定、compose_content解析失敗等 → `400 VALIDATION_ERROR` / `422 UNPROCESSABLE`
- `docker build`/`docker push`の失敗 → `422 BUILD_FAILED`(`docker`コマンドのstderrを含む)

### `POST /api/services/{id_or_name}/upload` — アップロードによる更新(`sahai service update`専用)

`multipart/form-data`。`POST /api/services/upload`(新規登録)の更新版。既存サービスのプロジェクトを現在のディレクトリの状態でビルド+push(上書き方式、要件定義書5章)する。`name`/`source_type`はここでは変更できない(対象サービスはパスの`{id_or_name}`で特定する)。

**リクエストパート**:

| パート名 | Content-Type | 内容 |
|---|---|---|
| `metadata` | `application/json` | `{ build_args?: { key: string, value: string }[], platform?: string }`(`name`を含まない点が`POST /api/services/upload`と異なる) |
| `archive` | `application/gzip` | プロジェクトディレクトリのtar.gz |

**処理**: 対象サービスの存在確認(無ければ`404`) → レジストリ資格情報の設定チェック → tar.gz展開 → アップロードされたプロジェクト構成(image型/compose型)が登録済みの`source_type`と一致するか検証(不一致なら`422`) → `docker build`/`docker push`(image型は`<service-name>:latest`を上書き。compose型は`build:`を持つ各サービスを`<service-name>-<composeサービス名>:latest`で上書き) → **compose型のみ**、新しい`compose_content`を`PUT /api/services/{id_or_name}`と同じ`service::update::update`ロジックへ渡し、6章のdiffロジックで`ServiceContainer`を同期する(既存コンテナの`ports`/`volumes`は維持される)。image型はDB上の`image`列を変更する必要がない(常に同じ`:latest`タグのため)。

**このエンドポイントもビルド完了まで同期的にブロックする**(`POST /api/services/upload`と同様、`DefaultBodyLimit`を500MBへ引き上げている)。

**レスポンス**: `200 OK`、ボディは更新後の`ServiceDetail`。

**エラー**:
- 対象サービスが存在しない → `404 NOT_FOUND`
- レジストリ資格情報未設定、アップロードされた構成と`source_type`の不一致、compose_content解析失敗等 → `400 VALIDATION_ERROR` / `422 UNPROCESSABLE`
- `docker build`/`docker push`の失敗 → `422 BUILD_FAILED`

**反映のタイミング**: `PUT /api/services/{id_or_name}`と同様、ビルドしたイメージ・保存した`compose_content`の実際の反映(コンテナ再作成)には別途`start`/`restart`が必要。`sahai service update`は`--deploy`指定時のみ自動で`restart`を呼ぶ。

### `GET /api/services` — 一覧

**クエリパラメータ**: なし(MVPスコープではフィルタ・ページネーションは設けない。数台〜十数台規模のため)

**レスポンス**: `200 OK`

```ts
{ services: Service[] }
```

### `GET /api/services/{id_or_name}` — 詳細

**レスポンス**: `200 OK`、ボディは`ServiceDetail`。

### `PUT /api/services/{id_or_name}` — メタデータ更新

**リクエストボディ**(すべて任意。**指定したフィールドのみ更新**し、省略したフィールドは変更しない):

```ts
type UpdateServiceRequest = {
  name?: string,
  image?: string,                     // source_type=image のサービスのみ指定可
  compose_content?: string,           // source_type=compose のサービスのみ指定可
  env_vars?: Record<string, string>,  // 指定時はオブジェクト全体を置き換え(マージしない)
  containers?: {                      // 指定時、各要素が指すコンテナのports/volumesを全置き換え(詳細は下記)
    name: string,
    ports?: { container_port: number, host_port: number, protocol?: "tcp"|"udp", is_http?: boolean }[],
    volumes?: { container_path: string }[],
  }[],
}
```

**`compose_content`変更と`containers`指定は独立した2つの仕組みであり、両者は次の順序で処理する**:

1. **`compose_content`が指定されている場合**: それをパースし、`ServiceContainer`を`name`で新旧突き合わせて6章「compose_contentの編集」のdiffロジック(新規追加/削除/継続)を適用する。これは`containers`フィールドの有無に関わらず行われる。`compose_content`が指定されていない場合、この差分検出は行わず既存の`ServiceContainer`集合をそのまま使う
2. **`containers`が指定されている場合**: 各要素の`name`は、**手順1適用後に実在する**`ServiceContainer`の名前でなければならない(存在しない名前を指定した場合は`VALIDATION_ERROR`。例えば同じリクエストで`compose_content`からサービスを削除しつつ、削除したサービス名を`containers`にも指定するのは不正)。該当する`ServiceContainer`の`ports`/`volumes`を、指定内容で**全置き換え**する(既存の`ServicePort`/`ServiceVolume`をいったん削除し、リクエスト内容で再作成。これらのレコードの`id`自体には実行時上の意味がなく、`container_id`と`service_id`のみが実体(Dockerコンテナ名・ボリュームパス)に影響するため、置き換えても副作用はない)
   - `source_type=image`のサービスの場合、`containers`は要素1件のみ許可し、`name`は`Service.name`と完全一致必須(POSTと同じ規則)

`is_http`がサービスにつき最大1件であることのチェックは、上記1・2適用後の最終状態に対して行う(違反時は`VALIDATION_ERROR`)。

`source_type`自体は本エンドポイントでは変更できない(スキーマに項目なし。変更したい場合はサービスを削除して再登録する)。

**副作用のタイミング**(要件定義書7章・12章と対応):
- `name`変更: **即時**。`subdomain`が連動して変わり、Traefikルートを直ちに書き換える。稼働中でも可能(コンテナ実体には影響しない)
- `image`/`compose_content`/`env_vars`/`containers`(ports/volumes)の変更: 保存のみ。**次回のstart/restartで**override・Traefikルートが再生成されて反映される

**レスポンス**: `200 OK`、ボディは更新後の`ServiceDetail`。

**エラー**: `name`重複 → `409`。バリデーション違反(`host_port`の衝突を含む。POSTと同じ検証を行う) → `400`。`source_type`と矛盾する`image`/`compose_content`指定 → `422`。

### `DELETE /api/services/{id_or_name}` — 削除

**クエリパラメータ**: `purge_volumes` (`true` | `false`、省略時`false`)

**処理**: 7章「サービス削除フロー」の通り、Traefikルート削除 → コンテナ/composeプロジェクト停止 → DBレコード削除(CASCADE)の順に実行。稼働中でも直接呼び出し可能。

**レスポンス**: `204 No Content`(成功時、ボディなし)

**エラー**: 途中(ルート削除・コンテナ停止)で失敗した場合、DBレコードは削除せず`500 INTERNAL_ERROR`(部分的な状態変化が起きている可能性があるため、レスポンスの`message`に失敗したステップを明記する)

### `POST /api/services/{id_or_name}/start`

**処理**: `docker pull` → `docker run`/`docker compose up -d --remove-orphans` → Traefikルート再生成(7章)。

Docker操作が失敗した場合は`status: "error"`とあわせて`last_error`にその標準エラー出力を保存し、レスポンスにも含める。`last_error`は起動が成功した時点でnullに戻る。`route_warning`が「起動はできたがルートが書けなかった」ことを表すのに対し、`last_error`は「起動そのものができなかった」ことを表す。

**冪等性**: 既に`status=running`のサービスに対しては**何もせず**そのまま`200`を返す(pull/run等のDocker操作は一切行わない)。「隠れた再起動」で呼び出し側が意図しないダウンタイムを起こさないための設計。既存の設定を反映させたい場合は明示的に`/restart`を呼ぶ。`status=stopped`または`status=error`の場合のみ実際にpull+runを実行する

**Traefikルート書き出し失敗時の扱い(`route_warning`)**: Dockerコンテナ自体の起動には成功したが、続くTraefikルートの書き出し(要件定義書7章)に失敗した場合、この呼び出し自体は`200`のまま`status: "running"`を返す(Docker操作は成功しているため。失敗にすると、既に`status=running`である以上は次の`/start`が冪等no-opで何もせず「再実行しても直らない」詰み状態になってしまう)。代わりにレスポンスの`route_warning`フィールドに理由と対処法(`/restart`を呼ぶよう促す文言)を積んで返す。値が無い(通常時)はキー自体を省略する。

**レスポンス**: `200 OK`、ボディは更新後の`ServiceDetail`(起動成功なら`status: "running"`、失敗なら`status: "error"`。Traefikルート書き出し失敗時のみ`route_warning`も含む)

### `POST /api/services/{id_or_name}/stop`

**処理**: `docker stop`/`docker compose down`。

**冪等性**: 既に`status=stopped`のサービスに対しても`200`を返す(no-op)。

**レスポンス**: `200 OK`、ボディは更新後の`ServiceDetail`(`status: "stopped"`)

### `POST /api/services/{id_or_name}/restart`

**処理**: stop→start(7章)。イメージ上書き後の再デプロイに使用。

**レスポンス**: `200 OK`、ボディは更新後の`ServiceDetail`

### `GET /api/services/{id_or_name}/stats`

**レスポンス**: `200 OK`(`status != "running"`のサービスに対しては`containers: []`を返す)

```ts
{
  containers: {
    id: number,
    name: string,
    cpu_percent: number,
    memory_usage_bytes: number,
    memory_limit_bytes: number,
  }[]
}
```

### `GET /api/services/{id_or_name}/logs` — コンテナログの配信(SSE)

要件定義書9章に対応。**このAPIだけJSONを1往復で返さず、`text/event-stream`を接続が切れるまで流し続ける。**

**クエリパラメータ**

| 名前 | 既定値 | 説明 |
|---|---|---|
| `container` | サービスの最初のコンテナ | 対象の`ServiceContainer.id` |
| `tail` | `200` | 接続時に送る直近の行数。`1`〜`5000` |

**レスポンス**: `200 OK`、`Content-Type: text/event-stream`。1行につき1イベントを送る。

```
event: line
data: {"stream":"stdout","timestamp":"2026-08-11T07:05:25.472263Z","message":"Listening on :3000"}
```

- `stream`: `"stdout"` | `"stderr"`
- `timestamp`: Dockerが記録した時刻。取得できない場合は`null`
- `message`: 行の内容(末尾の改行は除去済み)

読み出しが継続できなくなった場合は`error`イベントを1件送って接続を閉じる。

```
event: error
data: {"message":"コンテナが見つかりません"}
```

**認証は他のエンドポイントと同じ`Authorization: Bearer`**。ブラウザの`EventSource`はヘッダーを付けられないため、Web UIは`fetch`のストリームを読んで自前でSSEを解釈する。トークンをクエリ文字列に置く方式は採らない(URLがアクセスログ・プロキシ・ブラウザ履歴に残るため)。

**エラー**: 対象サービスが無ければ`404`、`container`が当該サービスのものでなければ`404`、`tail`が範囲外なら`400`。これらは接続を確立する前に通常のJSONエラーとして返す。

**停止中のサービスに対しても接続自体は成功する**が、差配はstopでコンテナごと削除するため、直後に「コンテナがありません」の`error`イベントを返して閉じることになる(要件定義書9章)。

### 初期設定・基本設定・DNS/証明書設定・レジストリ設定

`/api/services/*`とはリソースが異なるため、エンドポイント自体もパスが独立している。dns_provider/acme_emailは専用の`/api/settings/dns-provider`、registry_url/registry_username/registry_passwordは専用の`/api/settings/registry`にそれぞれ分離しているのは、保存操作の性質が全く異なるため(下記参照)。

#### `GET /api/setup` — 初期設定状況の確認

**認証不要**(未ログイン状態のWeb UIが、ログイン画面を出す前にこれを呼んで判定する)。

**レスポンス**: `200 OK`

```ts
{ configured: boolean }   // api_tokenが空でなければtrue
```

#### `POST /api/setup` — 初期設定の作成

**認証不要**。`configured: false`のとき(DBに設定行がまだ無い状態)のみ受け付ける。

**リクエストボディ**: `Settings`の`domain`・`https_redirect`・`api_token`(`dns_provider`/`acme_email`/`registry_url`/`registry_username`/`registry_password`はこの画面では入力させず、`cloudflare`/空文字列/Noneで仮置きする。後から「DNS/証明書設定」「レジストリ設定」画面で設定する)。

**処理**: バリデーション(`domain`・`api_token`必須) → DB新規作成(`registry_url`は空のまま保存され、内部の`apply_registry_url_default`が`domain`から`registry.sahai.<domain>`を自動生成する) → メモリ上の設定へ反映 → 管理画面用Traefikルートの初回書き出し(起動時点では`domain`が空でルートを書き出せないため、ここで初めて書き出す)。

**レスポンス**: `200 OK`、ボディは作成後の`Settings`(`api_token`を含むため、Web UIはこれを使ってログイン状態に遷移できる)。

**エラー**: 既に設定済み → `409 CONFLICT`。バリデーション違反 → `400 VALIDATION_ERROR`。

#### `GET /api/settings` / `PUT /api/settings` — 基本設定

**認証必須**。

**処理(PUT)**: リクエストボディは`Settings`の`domain`・`https_redirect`・`api_token`のみ(`dns_provider`/`acme_email`/`registry_url`/`registry_username`/`registry_password`を送っても無視され、既存の値がそのまま維持される。それぞれ専用の`/api/settings/dns-provider`・`/api/settings/registry`経由でのみ変更できる)。バリデーション → DB永続化 → メモリ上の設定へ即座に反映 → 管理画面用Traefikルート+全登録サービスのTraefikルートを再生成(`domain`/`https_redirect`の変更は全サービスのルートに影響するため)。**Traefikコンテナ自体の再作成は行わない**(下記DNS/証明書設定との違い)。

**冪等性に関する注意**: ルート再生成が一部のサービスで失敗しても、設定保存自体は失敗として扱わない(ログに警告を残すのみで`200`を返す)。

**レスポンス**: `200 OK`、ボディは`Settings`。

**エラー**: バリデーション違反(`domain`/`api_token`が空) → `400 VALIDATION_ERROR`。

#### `GET /api/settings/dns-provider` / `PUT /api/settings/dns-provider` — DNS/証明書設定

**認証必須**。基本設定とは別画面・別保存アクションにしている(要件定義書4章「HTTPS」参照)。

**処理(PUT)**: リクエストボディは`DnsConfig`。Traefikの`certificatesResolvers`は起動時の静的設定(CLI引数)としてしか渡せず、動的ファイルのホットリロード対象外のため、この保存操作だけは他と異なり**Traefikコンテナ自体の再作成**を伴う。処理順序: ①バリデーション(`dns_provider`・`acme_email`・各`credentials[].key`が空でないこと) → ②`.sahai.env`(`SAHAI_DATA_ROOT`直下)への書き込み(無ければディレクトリごと自動作成) → ③DB永続化 → ④メモリ上の設定へ反映 → ⑤bollard直接操作でTraefikコンテナを再作成(既存コンテナをinspectして設定を複製し、Envだけ`.sahai.env`の最新内容で組み立て直す。`traefik::container::recreate_traefik`)。

**注意(重要)**: 再作成対象は「今まさにこのリクエストを中継しているTraefikコンテナ自身」であるため、保存後の数秒〜(Windows/Docker Desktop環境では最大48秒程度)管理画面自体への接続が一時的に切れる。Web UI(`DnsConfigSection`)は保存後「再接続中」を表示しつつ`GET /api/settings/dns-provider`をポーリングし直し、接続が戻ったら「再接続を確認しました」に切り替える。

**レスポンス**: `200 OK`、ボディは保存後の`DnsConfig`。

**エラー**: バリデーション違反 → `400 VALIDATION_ERROR`。`.sahai.env`書き込み失敗・Traefik再作成失敗 → `500 INTERNAL_ERROR`(この時点でDB・ファイルへの書き込みは既に成功しているため、手動で`docker start <traefikコンテナ名>`を実行すれば復旧できる。②③の書き込みを⑤より先に行っているのはこのため)。

#### `GET /api/settings/registry` / `PUT /api/settings/registry` — レジストリ設定

**認証必須**。`sahai service create`(サーバー側build+push)がレジストリへpushする際に使う接続先・資格情報。以前は`SAHAI_REGISTRY_USERNAME`/`SAHAI_REGISTRY_PASSWORD`環境変数(`.env`)からのみ設定可能だったが、Web UIから編集・即時反映できるようにした。これらの環境変数は現在は初回シード専用(DBに行があれば無視される。他の設定項目の`seed_from_env`と同じパターン)。

**処理(PUT)**: リクエストボディは`RegistryConfig`の`registry_url`(省略可)・`registry_username`・`registry_password`。`registry_username`/`registry_password`は**両方null(未設定のまま) か 両方非null**のいずれかのみ許可し、片方だけの入力は`400 VALIDATION_ERROR`(空文字列はnullとして正規化されるため、両方空にすれば資格情報をクリアできる)。処理順序: ①バリデーション → ②`registry_url`省略時は`domain`から`registry.sahai.<domain>`を自動生成 → ③メモリ上の設定へ反映 → ④DB永続化 → ⑤`registry_username`/`registry_password`が両方設定されていれば同期的に`docker login`を試みる。

**DNS/証明書設定との重要な違い**: `docker login`は同期的にすぐ終わる軽い処理で接続断も起こさないため、DNS設定のような再接続ポーリングの仕組みは無い。また、**`docker login`が失敗しても`200 OK`を返し、DB保存自体は成功として扱う**(失敗理由は`login_warning`フィールドに載せる。設定を先に保存しておいてから資格情報を後で修正する、といった使い方ができる)。

**レスポンス例(docker login失敗時)**:
```json
{
  "registry_url": "registry.sahai.example.com",
  "registry_username": "myuser",
  "registry_password": "wrong-password",
  "login_warning": "レジストリへのログインに失敗しました: unauthorized: authentication required"
}
```
成功時は`login_warning`キー自体が省略される。

**レスポンス**: `200 OK`、ボディは`RegistryConfig`(保存済みパスワードも平文のまま含まれる。`DnsConfig.credentials`と同じ方針)。

**エラー**: `registry_username`/`registry_password`の片方のみ指定 → `400 VALIDATION_ERROR`。

### `GET /api/not-service` — Not Serviceページ用の公開情報API

要件定義書6章「非HTTPサービス・未登録サブドメインのルーティング」に対応。`/api/services/*`とは異なり**認証不要**(Web UIの未ログイン状態からも呼ぶため)。

**クエリパラメータ**: `host`(必須。例: `mysql.example.com`。`window.location.hostname`をそのまま渡す想定)

このAPIを叩くNot Serviceページ自体へは、sahai-serverのSPAフォールバックが`/not-service`へリダイレクトすることで到達する(要件定義書6章「Not Serviceページへの誘導」)。

**レスポンス**: 常に`200 OK`(見つからない場合もHTTPエラーにはしない。Docker操作の成否を`status`フィールドで表現するのと同じ設計方針)

```ts
{
  found: boolean,
  name?: string,               // found=trueのときのみ
  ports?: {
    host_port: number,
    container_port: number,
    protocol: "tcp" | "udp",
  }[],                          // found=trueのときのみ(空配列もありうる)
}
```

`host`に該当するサービスが存在しない場合(未登録サブドメイン)は`{ found: false }`のみを返す。

## 4. このドキュメントで新たに決定した事項(要件定義書からの補足)

要件定義書は概略のみを扱うため、実装可能な粒度に落とし込む過程で以下を決定した。要件定義書の趣旨と矛盾しないよう配慮したが、明示されていなかった点なので確認をお願いしたい。

- **`containers[].name`をimage型でも必須にし、`Service.name`との完全一致を要求する**(6章の「image型もcompose型と同じ構造で扱う」方針の延長)。compose型は部分集合でよく、未記載のサービスはports/volumes空で作成される(6章の「推奨」というニュアンスを尊重)
- **`compose_content`の変更(ServiceContainerの追加/削除)と`containers`の指定(ports/volumesの全置き換え)は独立した仕組みとして扱う**。前者は`compose_content`が指定されていれば`containers`の有無に関わらず発生する
- **PUTは指定フィールドのみ更新(部分更新)、ただし`containers`を指定した場合は該当コンテナのports/volumesは全置き換え**
- **`is_http`がサービスにつき最大1件であることをPOST/PUT双方の処理ステップで明示的にアプリ層検証する**(DBでは同一コンテナ内のみ強制されるため)
- **バリデーションエラーは`fields`配列で複数まとめて返す**(fail-fastにしない。フォーム入力のUXを優先)
- **start/stopは真に冪等**(既に目的の状態ならDocker操作を一切行わず200を返す。「隠れた再起動」による意図しないダウンタイムを避けるため)
- **Docker操作自体の失敗はHTTPエラーにせず`status: "error"`で表現する**(APIコールの成否とドメイン操作の成否を分離)
- **DELETEは`204 No Content`、start/stop/restart/PUTは更新後の`ServiceDetail`を返す**(呼び出し側が追加のGETを不要にするため)
