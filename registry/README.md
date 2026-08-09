# レジストリ認証情報の初回セットアップ

要件定義書3章の通り、`registry:2` の認証は `htpasswd` を使用する。単独ユーザー1アカウント分を初回セットアップ時に生成する。

**`setup.sh`/`setup.ps1`を使う場合、このセットアップは自動化されている。** `registry/auth/htpasswd`が未作成の状態で実行すると、対話モードでは次のメニューが表示される:

```
  1) auto(default)
  2) manual
>
```

- **auto**(既定、空Enterでも選択される): ユーザー名は固定値`sahai`、パスワードは十分に堅牢なランダム値(`openssl rand`)を自動生成する。生成した値で`auth/htpasswd`を作成し、同じ値を`sahai service create`/`update`(サーバー側build/push)用の資格情報(`PUT /api/settings/registry`)にも自動登録する。生成したユーザー名・パスワードはセットアップ完了時に一度だけ表示される(以後は表示されないため控えておくこと)。
- **manual**: レジストリURL・ユーザー名・パスワードを個別に入力する。URLを既定値(`registry.<ベースドメイン>`、空Enterでこの既定値になる)のままにした場合は同梱`registry:2`コンテナ用の`auth/htpasswd`も同じユーザー名・パスワードで作成する。既定値と異なるURLを入力した場合(同梱の`registry:2`ではなく外部レジストリを使う場合)は`auth/htpasswd`の生成をスキップし、`sahai service create`用の資格情報登録のみ行う。

既に`auth/htpasswd`が存在する場合(再インストール等)はこのステップ自体をスキップし、既存ファイルを再利用する。`sahai service create`用の資格情報を変更したい場合はWeb UIの「レジストリ設定」カードから行う。

`setup.sh`/`setup.ps1`を使わず手動でセットアップする場合は、以下を実行する:

```bash
mkdir -p auth
docker run --rm httpd:2.4-alpine htpasswd -Bbn <username> <password> > auth/htpasswd
```

生成した `auth/htpasswd` はコミットしない(`.gitignore`済み)。`compose.yaml` の `registry` サービスがこのディレクトリを読む。

利用側は事前に以下を実行しておく(`sahai container push` はレジストリ認証情報を保持せず、利用者のローカルDocker資格情報ストアに委ねる。要件定義書3章参照)。

```bash
docker login registry.sahai.example.com
```

## `sahai service create`(サーバー側build/push)用の資格情報

`sahai service create`(プロジェクトをサーバーへアップロードし、サーバー側でdocker build/pushを代行するコマンド)を使う場合は、上記とは別に**sahai-server自身の資格情報**が必要になる。`setup.sh`/`setup.ps1`のauto/manualメニューで自動的に設定されるため、通常はこの節を意識する必要はない。

この資格情報はWeb UIの「レジストリ設定」カード(`/settings`画面、`GET/PUT /api/settings/registry`)から**任意で**変更できる(パスワードローテーション、または同梱の`registry:2`ではない外部レジストリへ切り替えたい場合のみ使う拡張設定)。保存すると、sahai-serverがその場で`docker login`を実行し、以後のpushに使い回す(`.env`の編集やコンテナの`--force-recreate`は不要)。ログインに失敗しても設定自体は保存され、失敗理由は画面上に警告として表示される。

`SAHAI_REGISTRY_USERNAME`/`SAHAI_REGISTRY_PASSWORD`環境変数(`compose.yaml`のsahai-serverサービス参照)は、DBがまだ空の初回起動時にのみ読まれる後方互換のシード用になった(`setup.sh`/`setup.ps1`はこれらの環境変数ではなく`PUT /api/settings/registry`を直接呼ぶため使用しない)。以降はWeb UIで保存した値がDBの正となり、これらの環境変数を変更しても反映されない。

つまり2つの資格情報ストアが併存する: 利用者のローカルDocker資格情報ストア(`container push`用)と、sahai-serverコンテナ内のDocker資格情報ストア(`service create`用、Web UIから設定)。`setup.sh`/`setup.ps1`のautoモードは両方に同じ値を自動設定するため、通常は意識せず同じ資格情報のまま使える。
