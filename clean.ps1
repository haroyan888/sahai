#Requires -Version 7.0
<#
.SYNOPSIS
  差配(Sahai)の状態を初期化するスクリプト(Windows用)。
  セットアップをやり直せるよう、DB・設定・証明書・登録済みサービスをすべて消す。

.DESCRIPTION
  設定ファイルだけを消してDBを残すと「セットアップ済みだがトークンが分からない」
  状態になり復旧できないため、このスクリプトは常に一括で消す。

  ただしTLS証明書(traefik/acme)は既定で残す。Let's Encryptには「同じ識別子の組に
  対して7日間で5枚」という発行上限があり、消すとやり直すたびに1枚消費して
  数日間再取得できなくなるため。証明書は残っていてもセットアップをやり直せる。

.PARAMETER Yes
  確認プロンプトを出さずに実行する。

.PARAMETER CliConfig
  CLIの接続先設定(~/.config/sahai/config.toml)も削除する。

.PARAMETER Acme
  取得済みのTLS証明書も削除する。既定では残す。
#>

param(
    [switch]$Yes,
    [switch]$CliConfig,
    [switch]$Acme
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

$ComposeFile = Join-Path $ScriptDir 'compose.yaml'
$DataRoot = '/var/sahai'
$SetupEnv = Join-Path $env:USERPROFILE '.config\sahai\setup.env'
$CliConfigPath = Join-Path $env:USERPROFILE '.config\sahai\config.toml'
# v0.1系までリポジトリ直下に置いていたhtpasswd。現在は$DataRoot/registry-auth配下に
# あり上のDataRoot削除で一緒に消えるが、既存環境に残っていることがあるので拾う
$LegacyHtpasswdFile = Join-Path $ScriptDir 'registry\auth\htpasswd'
$LegacyEnvFile = Join-Path $ScriptDir '.env'

function Write-Log { param([string]$Message) Write-Host $Message }
function Die { param([string]$Message) Write-Error $Message; exit 1 }

# PowerShellの実行ポリシーが厳しいと `.\clean.ps1` を直接実行できない。
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
    Write-Log "PowerShellの実行ポリシーが $persistent のため、次回から .\clean.ps1 を直接実行できません。"
    $answer = Read-Host "現在のユーザーのみ RemoteSigned に変更しますか? [y/N]"
    if ($answer -match '^(y|Y|yes|YES)$') {
        Set-ExecutionPolicy -Scope CurrentUser RemoteSigned -Force
        Write-Log "変更しました(CurrentUser = RemoteSigned)。"
    } else {
        Write-Log "変更しませんでした。次回は次のように実行してください:"
        Write-Log "  pwsh -ExecutionPolicy Bypass -File .\clean.ps1"
    }
    Write-Log ""
}


Confirm-ScriptExecutionPolicy

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) { Die "Dockerが見つかりません。" }
try { docker info | Out-Null } catch { Die "Dockerデーモンに接続できません。Docker Desktopを起動してください。" }

Write-Log "以下を削除します:"
Write-Log "  - 土台のコンテナ(traefik / sahai-server / registry)"
Write-Log "  - sahaiが起動した全サービスのコンテナ(svc-*)"
Write-Log "  - $DataRoot 配下(DB・レジストリのイメージと認証情報・サービスのボリューム)"
if (Test-Path $LegacyHtpasswdFile) { Write-Log "  - $LegacyHtpasswdFile(旧バージョンの置き場)" }
Write-Log "  - $SetupEnv"
if ($CliConfig) { Write-Log "  - $CliConfigPath" }
if ($Acme) { Write-Log "  - $DataRoot/traefik/acme(取得済みのTLS証明書)" }
Write-Log ""
Write-Log "サービスの永続化データも消えます。元に戻せません。"
if (-not $Acme) {
    Write-Log "TLS証明書($DataRoot/traefik/acme)は残します。Let's Encryptの発行上限を"
    Write-Log "使い切らないためです。消す場合は -Acme を付けてください。"
}

if (-not $Yes) {
    $answer = Read-Host "続行しますか? [y/N]"
    if ($answer -notmatch '^(y|Y|yes|YES)$') {
        Write-Log "中止しました。"
        exit 0
    }
}

# 1. sahaiが起動したサービスのコンテナ(compose型はプロジェクトごと消えるようネットワークも)
Write-Log "サービスのコンテナを削除しています..."
$svcContainers = docker ps -aq --filter 'name=^svc-'
if ($svcContainers) { docker rm -f @svcContainers | Out-Null }
$svcNetworks = docker network ls -q --filter 'name=^svc-'
if ($svcNetworks) { docker network rm @svcNetworks 2>$null | Out-Null }

# 2. 土台のコンテナ
if (Test-Path $ComposeFile) {
    Write-Log "土台のコンテナを停止しています..."
    docker compose -f $ComposeFile down --remove-orphans 2>$null | Out-Null
}

# 3. データルート。Docker DesktopではVM内部のパスでホストから直接消せないため、
#    コンテナ経由で削除する
docker run --rm -v "${DataRoot}:/target" alpine test -d /target 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Log "$DataRoot を削除しています..."
    if ($Acme) {
        docker run --rm -v "${DataRoot}:/target" alpine sh -c 'rm -rf /target/..?* /target/.[!.]* /target/*' 2>$null | Out-Null
    } else {
        # traefik/acmeだけ残す。findで1階層ずつ除外する
        docker run --rm -v "${DataRoot}:/target" alpine sh -c 'find /target -mindepth 1 -maxdepth 1 ! -name traefik -exec rm -rf {} + ; [ -d /target/traefik ] && find /target/traefik -mindepth 1 -maxdepth 1 ! -name acme -exec rm -rf {} + ; exit 0' 2>$null | Out-Null
    }
}

# 4. リポジトリ内・ホーム配下のファイル
foreach ($f in @($LegacyHtpasswdFile, $SetupEnv)) {
    if (Test-Path $f) { Remove-Item -Force $f }
}
if ($CliConfig -and (Test-Path $CliConfigPath)) { Remove-Item -Force $CliConfigPath }
if (Test-Path $LegacyEnvFile) {
    Write-Warning "旧バージョンが作った $LegacyEnvFile が残っています。秘匿値を含むため内容を確認のうえ削除してください。"
}

Write-Log ""
Write-Log "初期化しました。.\setup.ps1 でセットアップし直せます。"
if ((-not $CliConfig) -and (Test-Path $CliConfigPath)) {
    Write-Log "CLIの設定($CliConfigPath)は残しています。再セットアップ後はAPIトークンが変わるため 'sahai login' をやり直してください。"
}
