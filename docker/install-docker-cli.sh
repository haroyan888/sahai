#!/bin/sh
# Docker公式リポジトリを追加し、docker-ce-cli/docker-compose-pluginだけをインストールする。
# Dockerfileのdev/runtime両ステージから共有される(呼び出し元がca-certificates/curl/gnupgを
# 事前にインストールしておくこと)。
set -eu

install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian bookworm stable" \
    > /etc/apt/sources.list.d/docker.list
apt-get update
apt-get install -y --no-install-recommends docker-ce-cli docker-compose-plugin
rm -rf /var/lib/apt/lists/*
