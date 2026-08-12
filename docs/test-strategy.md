# 差配(Sahai) テスト戦略

[requirements.md](./requirements.md) の内容に対する受け入れ基準・テスト方針をまとめる。

## 1. テストレベルと自動化方針

| レベル | 対象 | 自動化 | ツール |
|---|---|---|---|
| 単体テスト(バックエンド) | I/Oを伴わない純粋なロジック(バリデーション・パス生成・diffアルゴリズム等) | ○ | `cargo test` |
| 単体テスト(フロントエンド) | Reactコンポーネント・ページの描画とユーザー操作 | ○ | `vitest` + Testing Library |
| DB統合テスト | SQLite実DBに対するクエリ・制約・トランザクション | ○ | `sqlx::test` + 一時SQLite |
| Docker統合テスト | bollard/`docker compose`/Traefik等、実Dockerデーモンを要する部分 | △ 一部自動化 | `cargo test -- --ignored`(実Dockerが必要なため既定では実行しない) |
| E2E | Web UIからの一連の操作フロー | ✕ 手動 | 実機での手動チェックリスト(4章) |

Docker統合テストのうち、**副作用を局所化できるもの(コンテナの起動・停止・命名、ボリュームの実到達性、Traefikコンテナ再作成、ルート書き出し失敗時の警告)は`#[ignore]`付きの自動テストとして用意している**。実Dockerデーモンが必要でCIでは動かせないため既定の`cargo test`からは外し、実機で明示的に実行する。

Web UI全体を通した操作フロー(E2E)は自動化しない。単独管理者向けの用途に対しブラウザ自動化の構築・維持コストが見合わないため、4章の手動チェックリストで担保する。

## 2. 単体テスト対象(純粋ロジック)

### 6章: サービス登録機能

- サービス名バリデーション: `^[a-z][a-z0-9-]{0,61}[a-z0-9]$`、2〜63文字。境界値(1文字、63文字、64文字、先頭数字、大文字混入、記号混入)。予約語(`sahai`)の完全一致拒否、予約語を含むだけの名前〈`sahai-app`等〉は許可されること。`registry`は予約語ではないこと
- composeサービス名バリデーション(12章と共通ロジック): Dockerリポジトリ名として有効な文字(`[a-z0-9._-]`)のみか。合成タグ`<service-name>-<composeサービス名>`が128文字以内か(境界値: 128文字ちょうど、129文字)
- ボリュームパス正規化: `/var/lib/mysql` → `var-lib-mysql`。先頭`/`なし・末尾`/`あり・連続スラッシュ等の異常入力
- ボリュームホストパス生成: `/var/sahai/services/<service_id>/<正規化パス>/` が `service_id` のみに依存し、`container_id`を含まないこと
- **データルートの2表現**: `SAHAI_HOST_DATA_ROOT`未設定なら`SAHAI_DATA_ROOT`と同値になること(本番の既定)。両者が食い違う場合、bindマウント元は`host_data_root`から、`purge_volumes`の削除対象は`sahai_data_root`から組み立てられること
- ホストポートバリデーション: `host_port`が1〜65535か(境界値: 0・1・65535)。差配自身が公開する予約ポート(80・443)でないか(`validation::validate_host_port`)。範囲そのものの制限は設けないため、20000番台以外の値が通ること
- **compose_contentの diff ロジック(重要)**: 新旧`compose_content`のcomposeサービス名集合から「新規追加」「削除」「継続」を正しく分類できるか
  - 全サービス継続(変更なし)
  - 1つ追加
  - 1つ削除
  - 追加と削除が同時に起きるケース(実質的なリネーム相当。「削除+新規追加」として扱われることを確認)
  - 空のcompose(サービスなし)からの追加、全削除
- **非HTTPサービス判定**: `is_http`ポートを持つ`ServiceContainer`が0件のとき、Not HTTP Serviceページへのルーティングを選ぶロジック
- **Not Serviceページへの振り分け判定**: SPAフォールバックがHostヘッダーから`/not-service`へリダイレクトすべきかを決める純粋関数。管理画面ホスト(`sahai.<domain>`)は対象外、それ以外のサブドメインは対象、ベースドメイン外のホスト名(`localhost`・生IP)と初期設定前(ベースドメイン空)は対象外、`/not-service`自身はリダイレクトしない(無限ループ防止)、Hostヘッダーのポート部分(`:8080`)を無視すること
- **`is_http`最大1件の検証ロジック**: サービス配下の全`ServiceContainer`の全`ServicePort`を横断して`is_http=1`が2件以上ある場合にエラーとするバリデーション関数

### 7章: 起動・停止・削除

- **override生成ロジック**: `compose_content`をパースし、`build:`を持つサービスにのみ`image:`を注入、全サービスに`container_name: svc-{id}`・ポート・ボリューム・`env_file`を注入する処理。生成後のYAML構造を期待値と比較
- **Traefikルート生成ロジック**: `is_http`ポートの有無によって、実サービスへのルート/Not HTTP Serviceページへのルートのどちらを生成するかの分岐
- **http/https両対応のルーター生成**: `https_redirect=false`のとき、1つの論理ルートにつきweb用(`tls`なし)とwebsecure用(`tls: {}`・`certResolver`なし)の2本が、同じルール・同じ優先度・同じ転送先で書き出されること。`true`のときはwebsecure用1本+リダイレクトルーターであること。websecure用の名前がサービス名と衝突しないこと
- コンテナ名・プロジェクト名の組み立て: `svc-{ServiceContainer.id}` / `svc-{Service.id}` が正しいIDから生成されること(サービス名の値に依存しないこと)

### 8章: ヘルスチェック

- 判定優先順位ロジック: `HEALTHCHECK`結果が存在する場合はそちらを優先、存在しない場合はRunning状態を採用する分岐関数
- 3回連続失敗→unhealthy、1回成功→healthy復帰のステートマシン(メモリ上のカウンタ管理)を、Docker接続をモック化して検証
  - 連続失敗2回→成功でカウンタリセットされるか
  - 3回目の失敗でunhealthyに遷移するか
  - unhealthy中に1回成功でhealthyに復帰するか

### API共通

- リクエストボディのバリデーション(必須項目欠落、型不正等)に対するエラーレスポンス

## 3. DB統合テスト対象(`sqlx::test` + 一時SQLite)

### スキーマ制約

- `services.name` のUNIQUE制約(重複登録がエラーになること)
- `services.subdomain`が`name`から正しく算出されて保存されること。`SAHAI_DOMAIN`が環境変数のためSQLiteのGENERATED列では表現できず、通常列としてアプリケーション層(`sahai_core::naming::subdomain_for`)がINSERT・name変更UPDATEの両方で明示的に計算・書き込む(要件定義書11章)。`name`更新時に`subdomain`も追従すること
- `services.source_type`が`image`/`compose_content`の相互排他を守ること(CHECK制約)
- `service_ports.host_port` が全`ServicePort`を通してUNIQUEであること(異なるサービス間でも重複不可)
- `service_containers` の `(service_id, name)` UNIQUE制約
- 各テーブルのCASCADE削除: `Service`削除で`ServiceContainer`→`ServicePort`/`ServiceVolume`まで連鎖して消えること

### 排他制御(7章)

- **同時実行での`host_port`重複検知**: 同じ`host_port`を指定した2つの登録リクエストを`tokio::join!`等で同時実行し、片方のみ成功・片方はエラーになることを確認(`BEGIN IMMEDIATE`トランザクションの直列化を検証)
- 同様のシナリオをポート更新(PUT)でも確認

### ライフサイクル・ステータス

- `status`の初期値が`stopped`であること
- `status='error'`はDB上有効な値として保存・取得できること(遷移トリガー自体はDocker操作を伴うため、本書2章のロジックテストと4章の手動確認でカバー)
- サービス削除フローのDB部分: `DELETE`相当の処理で関連テーブルが正しく消え、`purge_volumes`フラグの有無で挙動が変わることの確認(ボリューム実ディレクトリの削除自体はファイルシステム操作なので、この統合テストでは「削除対象パスの算出結果」まで確認し、実際の`rm`はモック化)

### compose_content編集(6章)

- PUTで`compose_content`を更新した際、新規追加された`ServiceContainer`が作成されること
- 削除されたサービスに対応する`ServiceContainer`が消え、`ServicePort`/`ServiceVolume`もCASCADEで消えること
- 継続するサービスの`ServiceContainer.id`が変わらないこと(既存の`ServicePort`/`ServiceVolume`がそのまま残ること)

## 4. 手動テストチェックリスト(Docker依存・実機確認)

実際のDockerホスト(または開発用VM)上で、以下を一通り確認する。`sahai`実装が一区切りつくごとに実施する回帰チェックリストとしても使う。

### 基本ライフサイクル

- [ ] image型サービスを登録し、`sahai container push` → Web UIから起動 → `docker ps`で`svc-{container_id}`という名前でコンテナが起動していることを確認
- [ ] Traefikルート書き込み先を意図的に塞いだ状態で起動し、`status: "running"`のまま`route_warning`にエラー内容が返ること・Web UIにアラート表示されることを確認(`cargo test -p sahai-server -- --ignored e2e_start_surfaces_warning_when_traefik_route_write_fails`で自動化済み)
- [ ] 起動したサービスのサブドメインにアクセスし、Traefik経由でHTTPS(Let's Encrypt証明書)でアクセスできることを確認
- [ ] compose型サービス(例: app + mysql構成)を登録・起動し、`docker compose ls`で`svc-{service_id}`というプロジェクト名になっていることを確認
- [ ] compose型サービスの各コンテナが`svc-{container_id}`という名前で起動していることを確認

### 非HTTPサービス・未登録サブドメイン

- [ ] `is_http`ポートを持たないサービス(例: DB単体)を登録・起動し、サブドメインへアクセスするとNot HTTP Serviceページが表示され、登録済みポート一覧が表示されることを確認
- [ ] 未登録のサブドメイン(例: `nosuch.<domain>`)へアクセスし、**ログイン画面ではなく**「サービスが見つかりません」が表示されることを確認(`/not-service`へリダイレクトされること・ページのJS/CSSが正しく読み込まれ白画面にならないこと)
- [ ] 上記の状態で`sahai.<domain>`へアクセスし、管理画面(ログイン画面)が従来どおり表示されることを確認
- [ ] `SAHAI_HTTPS_REDIRECT=false`で、上記2つを**`http://`と`https://`の両方**で行い同じ画面になることを確認(httpsは自己署名証明書の警告を許容する)。`https://`が404になる場合はwebsecure用ルーターが書き出されていない
- [ ] `SAHAI_HTTPS_REDIRECT=true`で、`http://`が443へ301リダイレクトされることを確認

### ヘルスチェック

- [ ] `HEALTHCHECK`命令ありのイメージで、意図的に異常を起こし(例: プロセスkill)、10秒間隔・3回失敗でunhealthy表示になることを確認
- [ ] `HEALTHCHECK`命令なしのイメージで、コンテナをkillした場合にRunning状態の変化で異常検知されることを確認
- [ ] compose型サービスで、1コンテナだけ異常にした場合、そのコンテナのみunhealthy表示になり他は正常表示のままであることを確認
- [ ] サービスをstopした際、UI上「停止中」と一律表示されること

### サービス名変更(稼働中)

- [ ] 稼働中のサービスの名前をWeb UIから変更し、`docker ps`で**コンテナが再作成されずコンテナIDが変わらない**ことを確認
- [ ] 名前変更後、旧サブドメインにアクセスすると到達不能になり、新サブドメインで正しくアクセスできることを確認

### compose_content編集

- [ ] 稼働中のcompose型サービスの`compose_content`を編集してサービスを1つ追加し、restart後に新コンテナが起動することを確認
- [ ] 同様にサービスを1つ削除し、restart後に`--remove-orphans`で古いコンテナが片付くことを確認
- [ ] 削除→同じコンテナ内パスで別サービスとして追加、を行い、ボリュームデータが引き継がれる(消えない)ことを確認

### 削除フロー

- [ ] 稼働中のサービスを削除し、Traefikルート削除→コンテナ停止→DB削除の順に処理されること(削除中に旧サブドメインへアクセスして早い段階で到達不能になることを確認)
- [ ] `purge_volumes=false`(デフォルト)で削除した場合、`/var/sahai/services/<id>/`が残ることを確認
- [ ] `purge_volumes=true`で削除した場合、`/var/sahai/services/<id>/`が削除されることを確認

### CLI

- [ ] `sahai login`でトークン保存、`sahai service list`等が動作すること
- [ ] Web UI未登録のサービス名で`sahai container push`を実行し、エラーで案内が出ること
- [ ] compose型で不正な文字種のサービス名を含む`compose_content`を登録しようとした際、Web UI登録時点でエラーになること(CLI push時まで遅延しないこと)

### Control planeのデプロイ

- [ ] Control planeコンテナを`/var/sahai`をホストと同一パスでマウントして起動し、bollard/`docker compose`経由で作成したボリュームがホスト側の期待パスに実在することを確認(Docker-out-of-Dockerのパス整合性)
- [ ] 開発構成(`SAHAI_HOST_DATA_ROOT`あり)で、ボリューム付きサービスを起動→`./data/services/<id>/<正規化パス>`がホストに出ること、ホストで書いたファイルがコンテナから読めること、`purge_volumes=true`での削除で実際に消えることを確認(2つのデータルート表現が同じ場所を指していることの検証)

## 5. 実行方法

```bash
# バックエンド: 単体テスト + DB統合テスト
# (sqlx::testが一時DBを用意するため、開発機の既存sahai.sqlite3には影響しない)
cargo test --workspace
```

```bash
# フロントエンド: Reactコンポーネントの単体テスト
cd web && npm test
```

```bash
# Docker統合テスト(実Dockerデーモンが必要。既定のcargo testからは除外されている)
cargo test -p sahai-server -- --ignored
```

手動チェックリスト(4章)は、リリース前に開発用Dockerホストで一通り実施する。チェック状態はこのファイルを編集して記録せず(コミット差分がノイズになるため)、実施したかどうかのみをリリース時に確認する。

`--ignored`側のテストは実際にコンテナを起動・削除し、Traefikコンテナの再作成も行う。**稼働中のsahai環境では実行しない**こと。
