#Requires -Version 7.0
<#
.SYNOPSIS
  差配(Sahai)の初回セットアップ〜起動を自動化するスクリプト(Windows用)。
  対象は本番用 compose.yaml のみ(dev.compose.yamlは対象外)。

.DESCRIPTION
  非対話実行したい場合は環境変数 SAHAI_SETUP_NONINTERACTIVE=1 を設定し、
  併せて必要な値(SAHAI_SETUP_DOMAIN 等)を環境変数で渡すこと。
  Windows版はsystemd相当のサービス化は行わない(Docker Desktop上での起動用)。

.NOTES
  デバッグ目的でも Set-PSDebug -Trace を使わないこと。パスワード・APIトークン・
  DNS認証情報がトレース出力に出てしまう。
#>

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

$ComposeFile = Join-Path $ScriptDir 'compose.yaml'
# 再実行時にAPIトークンを再利用するためだけの控え。設定値の正はDB側にあり、
# ここには他の値を書かない。CLIのconfig.tomlと同じ配置方針(setup.shと同一の相対パス)
$EnvFile = Join-Path $env:USERPROFILE '.config\sahai\setup.env'
# 旧バージョンがリポジトリ直下に作っていた同等ファイル。トークンの引き継ぎのみに使う
$LegacyEnvFile = Join-Path $ScriptDir '.env'
$script:LegacyEnvFileMigrated = $false
$DataRoot = '/var/sahai'
# 生成物はすべてデータルート配下に集約する(container-design.md 3章)。
# ここはdockerdが作るroot所有のディレクトリで、Windowsホストからは直接触れない
# (Docker DesktopではVM内)。読み書きはコンテナ経由で行う
$HtpasswdDir = "$DataRoot/registry-auth"
# v0.1系までリポジトリ直下に置いていた同等ファイル。移行のためだけに参照する
$LegacyHtpasswdFile = Join-Path $ScriptDir 'registry\auth\htpasswd'

function Write-Log { param([string]$Message) Write-Host $Message }
function Write-Warn2 { param([string]$Message) Write-Warning $Message }
function Die { param([string]$Message) Write-Error $Message; exit 1 }

# htpasswdの置き場($HtpasswdDir)はroot所有かつVM内にあるため、ホスト側からは
# 読み書きできない。コンテナ内から扱う(clean.ps1がデータルートの削除で使う手と同じ)
function Test-Htpasswd {
    docker run --rm -v "${HtpasswdDir}:/auth" httpd:2.4-alpine test -f /auth/htpasswd 2>$null | Out-Null
    return ($LASTEXITCODE -eq 0)
}

# パスワードは標準入力から渡す
# (引数に置くとプロセス一覧やdocker inspectのCmdに平文で残るため)
function Write-Htpasswd {
    param([string]$User, [string]$Password)
    $Password | docker run --rm -i -v "${HtpasswdDir}:/auth" httpd:2.4-alpine `
        sh -c 'htpasswd -Bni "$1" > /auth/htpasswd' sh $User
    if ($LASTEXITCODE -ne 0) { Die "htpasswdの作成に失敗しました。" }
}

# 空のhtpasswdを置く。ファイルが存在しないままbind mountするとDockerが
# ディレクトリを自動作成してしまい、registryコンテナの起動自体が壊れる
function Write-EmptyHtpasswd {
    docker run --rm -v "${HtpasswdDir}:/auth" httpd:2.4-alpine sh -c ': > /auth/htpasswd'
    if ($LASTEXITCODE -ne 0) { Die "htpasswdの作成に失敗しました。" }
}

# リポジトリ直下(旧)からデータルート配下(新)へhtpasswdを引き継ぐ。
# これをしないと、既存環境の更新時にregistryが認証ファイルを見失い
# 「htpasswd is missing, provisioning with default user」で勝手にランダムな
# 資格情報を作ってしまい、既存のdocker loginが通らなくなる
function Move-LegacyHtpasswd {
    if (-not (Test-Path $LegacyHtpasswdFile)) { return }
    if (Test-Htpasswd) { return }
    Get-Content -Raw -Path $LegacyHtpasswdFile |
        docker run --rm -i -v "${HtpasswdDir}:/auth" httpd:2.4-alpine sh -c 'cat > /auth/htpasswd'
    if ($LASTEXITCODE -eq 0) {
        Write-Log "$LegacyHtpasswdFile を $HtpasswdDir/htpasswd へ移行しました。"
        Write-Log "移行元のファイルは不要です。内容を確認のうえ削除してください。"
    } else {
        Write-Warn2 "htpasswdの移行に失敗しました。レジストリの認証が効かなくなる可能性があります。"
    }
}

# PowerShellの実行ポリシーが厳しいと `.\setup.ps1` を直接実行できない。
# このスクリプトが動いている時点で今回の実行自体は許可されているが
# (`pwsh -ExecutionPolicy Bypass -File ...` で起動した場合など)、恒久設定が
# Restricted/AllSignedのままだと次回もまた弾かれるため、同意を得て緩める。
function Confirm-ScriptExecutionPolicy {
    $blocking = @('Restricted', 'AllSigned')

    # グループポリシー由来のスコープはSet-ExecutionPolicyで上書きできない
    foreach ($scope in @('MachinePolicy', 'UserPolicy')) {
        if ((Get-ExecutionPolicy -Scope $scope) -in $blocking) {
            Write-Warning "実行ポリシーがグループポリシーで制限されています。直接実行するには管理者に確認してください。"
            return
        }
    }

    # Processスコープ(今回の実行限りの一時設定)を除いた、次回以降に効く値
    $persistent = @('CurrentUser', 'LocalMachine') |
        ForEach-Object { Get-ExecutionPolicy -Scope $_ } |
        Where-Object { $_ -ne 'Undefined' } |
        Select-Object -First 1
    if (-not $persistent) { $persistent = 'Restricted' }
    if ($persistent -notin $blocking) { return }

    Write-Log ""
    Write-Log "PowerShellの実行ポリシーが $persistent のため、次回から .\setup.ps1 を直接実行できません。"
    $answer = Read-Host "現在のユーザーのみ RemoteSigned に変更しますか? [y/N]"
    if ($answer -match '^(y|Y|yes|YES)$') {
        Set-ExecutionPolicy -Scope CurrentUser RemoteSigned -Force
        Write-Log "変更しました(CurrentUser = RemoteSigned)。"
    } else {
        Write-Log "変更しませんでした。次回は次のように実行してください:"
        Write-Log "  pwsh -ExecutionPolicy Bypass -File .\setup.ps1"
    }
    Write-Log ""
}


function Invoke-Compose {
    param([string[]]$ComposeArgs)
    & docker compose -f $ComposeFile @ComposeArgs
    if ($LASTEXITCODE -ne 0) { throw "docker compose $($ComposeArgs -join ' ') failed" }
}

# sahai-serverコンテナ内部からlocalhost:8080を叩く。ポート非公開のため
# ホストから直接到達する手段が無く、かつDocker Desktopではコンテナのブリッジ
# IPにホストから直接到達できないことが多いため、`docker compose exec`経由で
# コンテナ内部から自分自身を叩く方式に統一する(Dockerfileのruntimeステージには
# docker-cliインストールの依存としてcurlが含まれている)。
function Invoke-ApiGet {
    param([string]$Path, [string[]]$ExtraArgs = @())
    $curlArgs = @('exec', '-T', 'sahai-server', 'curl', '-fsS') + $ExtraArgs + @("http://localhost:8080$Path")
    $result = & docker compose -f $ComposeFile @curlArgs 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $result
}

function Invoke-ApiBody {
    param([string]$Method, [string]$Path, [string]$Body, [string[]]$ExtraArgs = @())
    $curlArgs = @('exec', '-T', 'sahai-server', 'curl', '-fsS', '-X', $Method, "http://localhost:8080$Path",
                  '-H', 'Content-Type: application/json', '-d', '@-') + $ExtraArgs
    $result = $Body | & docker compose -f $ComposeFile @curlArgs 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $result
}

# --- setup.env読み書きユーティリティ ---

function Get-EnvValueFrom {
    param([string]$File, [string]$Key)
    if (-not (Test-Path $File)) { return $null }
    $line = Get-Content $File | Where-Object { $_ -match "^$Key=" } | Select-Object -Last 1
    if ($null -eq $line) { return $null }
    return ($line -replace "^$Key=", '')
}

function Get-EnvValue {
    param([string]$Key)
    return (Get-EnvValueFrom -File $EnvFile -Key $Key)
}

function Set-EnvValue {
    param([string]$Key, [string]$Value)
    $isNew = -not (Test-Path $EnvFile)
    if ($isNew) {
        $parent = Split-Path -Parent $EnvFile
        if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    }
    if (-not $isNew) {
        $content = Get-Content $EnvFile
        if ($content -match "^$Key=") {
            $content = $content | ForEach-Object { if ($_ -match "^$Key=") { "$Key=$Value" } else { $_ } }
            Set-Content -Path $EnvFile -Value $content -Encoding utf8NoBOM
        } else {
            Add-Content -Path $EnvFile -Value "$Key=$Value" -Encoding utf8NoBOM
        }
    } else {
        Set-Content -Path $EnvFile -Value "$Key=$Value" -Encoding utf8NoBOM
    }
}

function ConvertFrom-SecureStringPlain {
    param([System.Security.SecureString]$Secure)
    return [System.Net.NetworkCredential]::new('', $Secure).Password
}

# APIトークン・レジストリパスワード等、十分に堅牢なランダム値が必要な箇所で共通利用する。
function New-RandomSecret {
    $bytes = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return -join ($bytes | ForEach-Object { $_.ToString('x2') })
}

# ============================================================
# 0. 前提条件チェック
# ============================================================
function Step0-CheckPrerequisites {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Die "Docker Desktopが見つかりません: https://www.docker.com/products/docker-desktop/"
    }
    try { docker compose version | Out-Null } catch {
        Die "docker compose v2が見つかりません。Docker Desktopを最新化してください。"
    }
    try { docker info | Out-Null } catch {
        Die "Dockerデーモンに接続できません。Docker Desktopを起動してください。"
    }
}

# ============================================================
# 1. レジストリ資格情報の決定(htpasswd作成 + PUT /api/settings/registry用の値を確定)
# ============================================================
# 値の決定はここで1回だけ行う(auto/manualの選択、URL/ユーザー名/パスワードの確定)。
# htpasswdファイルはdocker compose up(Step4)より前に存在しなければregistryコンテナの
# 認証が機能しないため、ここで書き出す。一方PUT /api/settings/registryはAPIトークン
# 確定・サーバー起動より後でなければ呼べないため、DBへの登録はStep10で行う
# (ここで確定した値をハッシュテーブルで返し、Mainが引き回す)。
#
# 戻り値のMode:
#   provisioned    - 今回新たに値を決定した(Step10でDB登録する)
#   reuse-existing - 既存のhtpasswdを再利用した(平文パスワードが分からないためDB登録はしない)
#   skip           - SAHAI_SETUP_SKIP_REGISTRY_SETTINGS=1で丸ごとスキップした
function Step1-ConfigureRegistry {
    Move-LegacyHtpasswd

    if (Test-Htpasswd) {
        Write-Log "$HtpasswdDir/htpasswd は既存のものを再利用します。"
        Write-Log "変更したい場合はWeb UIの「レジストリ設定」から行えます。"
        return @{ Mode = 'reuse-existing' }
    }

    if ($env:SAHAI_SETUP_SKIP_REGISTRY_SETTINGS -eq '1') {
        Write-Log "レジストリ設定をスキップしました(SAHAI_SETUP_SKIP_REGISTRY_SETTINGS=1)。"
        return @{ Mode = 'skip' }
    }

    # $HtpasswdDir はbindマウント時にdockerdが自動作成するため、ここでは作らない

    $mode = $null
    if ($env:SAHAI_SETUP_REGISTRY_URL -or $env:SAHAI_SETUP_REGISTRY_AUTH_USER -or $env:SAHAI_SETUP_REGISTRY_AUTH_PASSWORD) {
        # 非対話向けの環境変数指定が1つでもあればmanual相当として扱う
        $mode = 'manual'
    } elseif ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') {
        $mode = 'auto'
    } else {
        Write-Log ""
        Write-Log "レジストリ(registry:2)の資格情報を設定します:"
        Write-Log "  1) auto(default)"
        Write-Log "  2) manual"
        $choice = Read-Host ">"
        if ($choice -eq '2') { $mode = 'manual' }
        elseif ($choice -eq '' -or $choice -eq '1') { $mode = 'auto' }
        else { Die "1または2を入力してください。" }
    }

    $regUrl = ''
    $regUser = $null
    $regPass = $null

    if ($mode -eq 'manual') {
        # URLが既定値(registry.sahai.<domain>)かどうかの判定にdomainが要るが、通常domainは
        # Step6で初めて確定する(このstepより後)。ここで一度だけ確定させ、
        # $env:SAHAI_SETUP_DOMAINとして設定しておくことでStep6の二重プロンプトを防ぐ
        # (Step6-RunInitialSetupIfNeededは既に$env:SAHAI_SETUP_DOMAINを優先して使う)。
        if (-not $env:SAHAI_SETUP_DOMAIN) {
            $env:SAHAI_SETUP_DOMAIN = Read-Host "サービスのベースドメイン(例: example.com)"
            if (-not $env:SAHAI_SETUP_DOMAIN) { Die "ドメインを入力してください。" }
            Write-Log "  このホストを指すDNSレコードが2本必要です:"
            Write-Log "    *.$($env:SAHAI_SETUP_DOMAIN)       (管理画面と各サービス)"
            Write-Log "    *.sahai.$($env:SAHAI_SETUP_DOMAIN) (レジストリ)"
        }
        $defaultUrl = "registry.sahai.$($env:SAHAI_SETUP_DOMAIN)"

        $regUrl = $env:SAHAI_SETUP_REGISTRY_URL
        if (-not $regUrl) {
            if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') {
                $regUrl = $defaultUrl
            } else {
                $urlInput = Read-Host "レジストリURL [$defaultUrl]"
                $regUrl = if ($urlInput) { $urlInput } else { $defaultUrl }
            }
        }

        $regUser = $env:SAHAI_SETUP_REGISTRY_AUTH_USER
        if (-not $regUser) {
            if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') { Die "非対話モードですが SAHAI_SETUP_REGISTRY_AUTH_USER が未設定です。" }
            $regUser = Read-Host "レジストリ用ユーザー名"
            if (-not $regUser) { Die "ユーザー名を入力してください。" }
        }

        $regPass = $env:SAHAI_SETUP_REGISTRY_AUTH_PASSWORD
        if (-not $regPass) {
            if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') { Die "非対話モードですが SAHAI_SETUP_REGISTRY_AUTH_PASSWORD が未設定です。" }
            $secure1 = Read-Host "レジストリ用パスワード" -AsSecureString
            $secure2 = Read-Host "パスワード(確認)" -AsSecureString
            $p1 = ConvertFrom-SecureStringPlain $secure1
            $p2 = ConvertFrom-SecureStringPlain $secure2
            if ($p1 -ne $p2) { Die "パスワードが一致しません。" }
            $regPass = $p1
            Remove-Variable p1, p2, secure1, secure2 -ErrorAction SilentlyContinue
        }

        if ($regUrl -eq $defaultUrl) {
            Write-Htpasswd -User $regUser -Password $regPass
            Write-Log "$HtpasswdDir/htpasswd を作成しました(ユーザー名: $regUser)。"
        } else {
            # 中身が空でも有効なhtpasswdファイルとして扱われ、単に誰も認証できなくなるだけ
            # で済む。この同梱レジストリは使わない前提なので問題ない
            Write-EmptyHtpasswd
            Write-Log "レジストリURLが既定値($defaultUrl)と異なるため、同梱registryコンテナのhtpasswdは変更しません(未使用として空ファイルを作成)。"
        }
    } else {
        # auto: ユーザー名固定+パスワード自動生成。URLは空のままサーバー側の
        # apply_registry_url_defaultにregistry.sahai.<domain>を自動補完させる
        $regUrl = ''
        $regUser = if ($env:SAHAI_SETUP_REGISTRY_AUTH_USER) { $env:SAHAI_SETUP_REGISTRY_AUTH_USER } else { 'sahai' }
        $regPass = if ($env:SAHAI_SETUP_REGISTRY_AUTH_PASSWORD) { $env:SAHAI_SETUP_REGISTRY_AUTH_PASSWORD } else { New-RandomSecret }
        Write-Htpasswd -User $regUser -Password $regPass
        Write-Log "$HtpasswdDir/htpasswd を作成しました(ユーザー名: $regUser、パスワードは自動生成)。"
    }

    return @{ Mode = 'provisioned'; Url = $regUrl; User = $regUser; Password = $regPass }
}

# ============================================================
# 2. SAHAI_API_TOKENの生成
# ============================================================
function Step2-EnsureApiToken {
    $token = Get-EnvValue -Key 'SAHAI_API_TOKEN'
    if ($token) {
        Write-Log "既存のAPIトークンを再利用します。"
        return $token
    }
    # 旧バージョンがリポジトリ直下の.envに保存していたトークンを引き継ぐ
    $token = Get-EnvValueFrom -File $LegacyEnvFile -Key 'SAHAI_API_TOKEN'
    if ($token) {
        Write-Log "既存のAPIトークンを $LegacyEnvFile から引き継ぎます。"
        $script:LegacyEnvFileMigrated = $true
        return $token
    }
    return New-RandomSecret
}

# ============================================================
# 3. setup.envの作成/更新(SAHAI_API_TOKENのみ)
# ============================================================
function Step3-WriteSetupEnv {
    param([string]$ApiToken)
    Set-EnvValue -Key 'SAHAI_API_TOKEN' -Value $ApiToken
}

# ============================================================
# 4. 起動
# ============================================================
function Step4-ComposeUpBuild {
    # 公開イメージがあれば取得し、取得できなければソースからビルドする。
    # `up --pull always`の暗黙のフォールバックには頼らない。取得失敗時にビルドへ
    # 回るかどうかはdocker composeのバージョンによって変わり、古い版では
    # そのままエラー終了してしまうため。
    Write-Log "sahai-serverのイメージを取得しています..."
    & docker compose -f $ComposeFile pull sahai-server 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Log "  公開イメージを取得しました。"
    } else {
        Write-Log "  公開イメージを取得できませんでした。ソースからビルドします(数分かかります)..."
        Invoke-Compose @('build', 'sahai-server')
    }
    Write-Log "コンテナを起動しています..."
    Invoke-Compose @('up', '-d')
}

# ============================================================
# 5. sahai-serverの起動待ち
# ============================================================
function Step5-WaitForSahaiServerReady {
    Write-Log "sahai-serverの起動を待っています..."
    $deadline = (Get-Date).AddSeconds(120)
    while ($true) {
        $r = Invoke-ApiGet -Path '/api/setup'
        if ($null -ne $r) { break }
        if ((Get-Date) -gt $deadline) {
            Die "sahai-serverの起動待ちがタイムアウトしました(120秒)。'docker compose -f compose.yaml logs sahai-server' で原因を確認してください。"
        }
        Start-Sleep -Seconds 2
    }
    Write-Log "sahai-serverが起動しました。"
}

# ============================================================
# 6. 初回セットアップ(POST /api/setup)
# ============================================================
function Step6-RunInitialSetupIfNeeded {
    param([string]$ApiToken)

    $statusJson = Invoke-ApiGet -Path '/api/setup'
    $status = $statusJson | ConvertFrom-Json

    if ($status.configured) {
        Write-Log "既にセットアップ済みのため初期設定はスキップします。"
        # ApiTokenがDB上のトークンと一致しない場合(DBのデータだけ残った状態で
        # .envを削除・再生成した場合等)、この呼び出しは401で失敗しInvoke-ApiGetは
        # $nullを返す。$null | ConvertFrom-Jsonは例外になり分かりにくいため、
        # 先にnullチェックして原因を案内する。
        $settingsJson = Invoke-ApiGet -Path '/api/settings' -ExtraArgs @('-H', "Authorization: Bearer $ApiToken")
        if ($null -eq $settingsJson) {
            Die "既にセットアップ済みですが、APIトークンでの認証に失敗しました。$EnvFile のSAHAI_API_TOKENが、既存のDBに保存されているトークンと一致しているか確認してください(DBのデータだけ残ったままsetup.envを削除・再生成した場合等に発生します。正しいトークンが分からない場合は、Web UIから再発行するか、DBを含むデータをリセットして最初からセットアップし直してください)。"
        }
        $settings = $settingsJson | ConvertFrom-Json
        return $settings.domain
    }

    $domain = $env:SAHAI_SETUP_DOMAIN
    if (-not $domain) {
        if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') {
            Die "非対話モードですが SAHAI_SETUP_DOMAIN が未設定です。"
        }
        $domain = Read-Host "サービスのベースドメイン(例: example.com)"
        if (-not $domain) { Die "ドメインを入力してください。" }
    }

    $body = @{ domain = $domain; https_redirect = $true; api_token = $ApiToken } | ConvertTo-Json -Compress
    # 初期設定の先取りを防ぐため、サーバーが起動時に発行したワンタイムトークンの提示が要る。
    # ファイルはSAHAI_DATA_ROOT(compose.yamlで/var/sahai固定)直下にあり、
    # ホストからは読めないためコンテナ内部から取得する
    $setupToken = (& docker compose -f $ComposeFile exec -T sahai-server cat /var/sahai/setup-token) 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($setupToken)) {
        Die "セットアップトークンを取得できませんでした。'docker compose -f compose.yaml logs sahai-server' で発行状況を確認してください。"
    }
    $setupToken = $setupToken.Trim()

    Write-Log "初期設定を保存しています..."
    $resp = Invoke-ApiBody -Method 'POST' -Path '/api/setup' -Body $body `
        -ExtraArgs @('-H', "X-Sahai-Setup-Token: $setupToken")
    if ($null -eq $resp) { Die "初期設定(POST /api/setup)に失敗しました。" }

    return $domain
}

# ============================================================
# 7. DNS/証明書設定(PUT /api/settings/dns-provider)
# ============================================================
function Step7-ConfigureDnsAndTls {
    param([string]$ApiToken)

    $dnsProvider = $env:SAHAI_DNS_PROVIDER
    if (-not $dnsProvider) {
        if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') {
            Die "非対話モードですが SAHAI_DNS_PROVIDER が未設定です。"
        }
        $dnsProvider = Read-Host "DNSプロバイダ(legoが対応するプロバイダ名。例: cloudflare。一覧: https://go-acme.github.io/lego/dns/index.html)"
        if (-not $dnsProvider) { Die "DNSプロバイダを入力してください。" }
    }

    $acmeEmail = $env:SAHAI_ACME_EMAIL
    if (-not $acmeEmail) {
        if ($env:SAHAI_SETUP_NONINTERACTIVE -eq '1') {
            Die "非対話モードですが SAHAI_ACME_EMAIL が未設定です。"
        }
        $acmeEmail = Read-Host "Let's Encrypt通知先メールアドレス"
        if (-not $acmeEmail) { Die "メールアドレスを入力してください。" }
    }

    $credentials = @()
    if ($env:SAHAI_SETUP_DNS_CREDENTIALS) {
        foreach ($pair in ($env:SAHAI_SETUP_DNS_CREDENTIALS -split ',')) {
            if ($pair.Trim().Length -eq 0) { continue }
            $idx = $pair.IndexOf('=')
            if ($idx -lt 0) { continue }
            $k = $pair.Substring(0, $idx)
            $v = $pair.Substring($idx + 1)
            $credentials += @{ key = $k; value = $v }
        }
    } elseif ($env:SAHAI_SETUP_NONINTERACTIVE -ne '1') {
        Write-Log "選んだプロバイダ($dnsProvider)が要求する認証情報を入力してください"
        Write-Log "例: cloudflareなら CF_DNS_API_TOKEN(一覧: https://go-acme.github.io/lego/dns/index.html)"
        Write-Log "キー名を空EnterでDNS設定を終了します。"
        while ($true) {
            $key = Read-Host "  環境変数名(空Enterで終了)"
            if (-not $key) { break }
            $secure = Read-Host "  $key の値" -AsSecureString
            $value = ConvertFrom-SecureStringPlain $secure
            $credentials += @{ key = $key; value = $value }
            Remove-Variable value, secure -ErrorAction SilentlyContinue
        }
    }

    $body = @{ dns_provider = $dnsProvider; acme_email = $acmeEmail; credentials = $credentials } | ConvertTo-Json -Compress -Depth 5

    Write-Log "DNS/証明書設定を保存しています(最大1分ほどかかります)..."
    $resp = Invoke-ApiBody -Method 'PUT' -Path '/api/settings/dns-provider' -Body $body -ExtraArgs @('-H', "Authorization: Bearer $ApiToken", '--max-time', '90')
    if ($null -eq $resp) {
        Write-Warn2 "DNS/証明書設定の保存に失敗しました。認証情報を確認してください。"
        Write-Warn2 "詳細: docker compose -f compose.yaml logs sahai-server traefik"
        return $false
    }

    # dns_provider・acme_email・認証情報の保存先はDBと/var/sahai/.sahai.envであり
    # (sahai-serverが上記PUTの中で書く)、こちら側での控えは持たない
    return $true
}

# ============================================================
# 8. 証明書取得の確認
# ============================================================
function Test-CertificateIssuer {
    param([string]$Domain)
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient($Domain, 443)
        $callback = { param($s, $c, $ch, $e) $true }
        $ssl = New-Object System.Net.Security.SslStream($tcp.GetStream(), $false, $callback)
        $ssl.AuthenticateAsClient($Domain)
        $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($ssl.RemoteCertificate)
        $issuer = $cert.Issuer
        $ssl.Close(); $tcp.Close()
        return $issuer
    } catch {
        return $null
    }
}

function Step9-VerifyCertificate {
    param([string]$Domain)
    if (-not $Domain) {
        Write-Warn2 "ドメインが未確定のため証明書確認をスキップします。"
        return $false
    }

    for ($attempt = 1; $attempt -le 4; $attempt++) {
        $issuer = Test-CertificateIssuer -Domain $Domain
        if ($issuer -and ($issuer -notmatch 'TRAEFIK DEFAULT CERT')) {
            Write-Log "証明書のissuer: $issuer"
            return $true
        }
        if ($attempt -lt 4) {
            Write-Log "証明書取得を確認できませんでした。DNS-01チャレンジの完了を待っています($attempt/4)..."
            Start-Sleep -Seconds 30
        }
    }

    Write-Warn2 "警告: まだLet's Encrypt証明書を確認できません(Traefikの自己署名証明書のままか、接続不可)。"
    Write-Warn2 "以下を確認してください:"
    Write-Warn2 "  - DNS($Domain)がこのサーバーを指しているか(DNS伝播に数分〜数時間かかることがあります)"
    Write-Warn2 "  - DNSプロバイダのAPIトークンに正しい権限があるか(例: Cloudflareなら Zone:DNS:Edit)"
    Write-Warn2 "  - docker compose -f compose.yaml logs traefik でエラーの詳細を確認"
    return $false
}

# ============================================================
# 9. レジストリ資格情報のDB登録(sahai service create用)
# ============================================================
# Step1で決定した値をここで登録する(APIトークン確定・サーバー起動後でなければ
# PUT /api/settings/registryを呼べないため)。Step1で"provisioned"以外
# (reuse-existing/skip)だった場合は何もしない(reuse-existingは平文パスワードが
# 分からないため登録しようが無く、skipは意図的にスキップされている)。
function Step10-RegisterRegistryCredentials {
    param([string]$ApiToken, [hashtable]$Registry)

    if ($Registry.Mode -ne 'provisioned') { return }

    $body = @{ registry_url = $Registry.Url; registry_username = $Registry.User; registry_password = $Registry.Password } | ConvertTo-Json -Compress
    $resp = Invoke-ApiBody -Method 'PUT' -Path '/api/settings/registry' -Body $body -ExtraArgs @('-H', "Authorization: Bearer $ApiToken")

    if ($resp) {
        $respObj = $resp | ConvertFrom-Json
        if ($respObj.login_warning) {
            Write-Warn2 "警告: $($respObj.login_warning)"
            Write-Warn2 "(設定自体は保存されています。Web UIの「レジストリ設定」から再確認できます)"
        } else {
            Write-Log "レジストリ資格情報を登録しました。"
        }
    } else {
        Write-Warn2 "レジストリ資格情報の登録に失敗しました。"
    }
}

# ============================================================
# 10. 完了メッセージ
# ============================================================
function Step12-PrintSummary {
    param([string]$Domain, [string]$ApiToken, [hashtable]$Registry)
    Write-Log ""
    Write-Log "====================================================="
    Write-Log "sahai のセットアップが完了しました。"
    Write-Log ""
    Write-Log "  管理画面: https://sahai.$Domain"
    Write-Log "  APIトークン(この場だけの表示です。控えてください):"
    Write-Log "    $ApiToken"
    Write-Log "  (このトークンは $EnvFile にも保存されています。"
    Write-Log "   セットアップ再実行時の再利用にのみ使われます)"
    Write-Log ""
    if ($Registry.Mode -eq 'provisioned') {
        $regUrlDisplay = if ($Registry.Url) { $Registry.Url } else { "registry.sahai.$Domain" }
        Write-Log "  レジストリ資格情報(この場だけの表示です。控えてください):"
        Write-Log "    URL:        $regUrlDisplay"
        Write-Log "    ユーザー名: $($Registry.User)"
        Write-Log "    パスワード: $($Registry.Password)"
        Write-Log "  (sahai service create用は設定済みです。ローカルからpushする場合のみ"
        Write-Log "   docker login $regUrlDisplay を実行してください)"
    } else {
        Write-Log "  ローカルからpushする場合:"
        Write-Log "    docker login registry.sahai.$Domain"
    }
    Write-Log ""
    if ($script:LegacyEnvFileMigrated) {
        Write-Log "  【要対応】APIトークンを $LegacyEnvFile から引き継ぎました。"
        Write-Log "  このファイルには古い認証情報が平文で残っています。"
        Write-Log "  現在は未使用のため、確認のうえ削除してください:"
        Write-Log "    Remove-Item '$LegacyEnvFile'"
        Write-Log ""
    }
    Write-Log "  Windows版はsystemd相当のサービス化を行いません。Docker Desktopを"
    Write-Log "  起動したままにしておく運用を想定しています(Linux本番運用にはsetup.shを使ってください)。"
    Write-Log "====================================================="
}

function Main {
    Confirm-ScriptExecutionPolicy
    Step0-CheckPrerequisites
    $registry = Step1-ConfigureRegistry
    $apiToken = Step2-EnsureApiToken
    Step3-WriteSetupEnv -ApiToken $apiToken
    Step4-ComposeUpBuild
    Step5-WaitForSahaiServerReady
    $domain = Step6-RunInitialSetupIfNeeded -ApiToken $apiToken
    try { Step7-ConfigureDnsAndTls -ApiToken $apiToken | Out-Null } catch { Write-Warn2 $_ }
    try { Step9-VerifyCertificate -Domain $domain | Out-Null } catch { Write-Warn2 $_ }
    Step10-RegisterRegistryCredentials -ApiToken $apiToken -Registry $registry
    Step12-PrintSummary -Domain $domain -ApiToken $apiToken -Registry $registry
}

Main
