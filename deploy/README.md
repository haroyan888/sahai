# systemdサービスとしての導入

`sahai.service` は `docker compose -f compose.yaml up -d` / `down` をトリガーするだけの薄いsystemdユニットです。通常は `setup.sh` が対話確認のうえ自動で導入します(該当ステップで `sudo` 権限が必要です)。

## `setup.sh` を使わず手動で導入する場合

1. このディレクトリの `sahai.service` をコピーし、プレースホルダを実際の値に置換します。

   ```bash
   REPO_DIR="/opt/sahai"          # このリポジトリの実際のクローン先の絶対パス
   DOCKER_BIN="$(command -v docker)"

   sed -e "s#__SAHAI_REPO_DIR__#${REPO_DIR}#g" \
       -e "s#__DOCKER_BIN__#${DOCKER_BIN}#g" \
       deploy/sahai.service > /tmp/sahai.service
   ```

   `REPO_DIR` はこのリポジトリの実際のクローン先の絶対パスを指定してください(`compose.yaml`の場所を特定するために使うだけで、`.env`側に対応する環境変数は不要です)。

2. `/etc/systemd/system/` に配置し、有効化します。

   ```bash
   sudo install -m 644 /tmp/sahai.service /etc/systemd/system/sahai.service
   sudo systemctl daemon-reload
   sudo systemctl enable sahai
   sudo systemctl start sahai
   ```

## 運用コマンド

```bash
sudo systemctl status sahai      # 状態確認
sudo systemctl start sahai       # 起動(docker compose up -d相当)
sudo systemctl stop sahai        # 停止(docker compose down相当、コンテナごと削除される)
sudo systemctl restart sahai
journalctl -u sahai -f           # ログ確認(compose自体の標準出力のみ。各コンテナのログは
                                  # 別途 `docker compose -f compose.yaml logs -f` を使う)
```

## 注意点

- `ExecStop` は `docker compose down` を実行するため、サービス停止時にコンテナは完全に削除されます(イメージ・ボリュームは残ります)。ホストの再起動等でシステムが `sahai.service` を停止させた場合も同様です。
- `Type=oneshot` + `RemainAfterExit=yes` を使っているため、`docker compose up -d` プロセス自体はすぐ終了し、systemdは「起動済み」として扱います。コンテナの異常終了はsystemd側では検知されません(Docker自体の `restart: unless-stopped` ポリシーに任せる設計)。
- アンインストールする場合:

  ```bash
  sudo systemctl disable --now sahai
  sudo rm /etc/systemd/system/sahai.service
  sudo systemctl daemon-reload
  ```
