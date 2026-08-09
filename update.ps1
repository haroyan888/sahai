<#
.SYNOPSIS
  差配(Sahai)を最新版へ更新する(Windows用)。

.DESCRIPTION
  設定・証明書・サービスの永続化データには一切触らない。土台の3コンテナ
  (traefik / sahai-server / registry)を新しいイメージで作り直すだけ。

  更新中も登録済みサービス(svc-*)は動き続ける。これらはcomposeの管理外であり、
  土台の作り直しでは停止しないため。止まるのは管理画面とレジストリだけ。

  起動時にDBマイグレーションが走る。失敗すると起動できず、SQLiteには
  ロールバックが無いため、事前にDBのバックアップを取る。

.PARAMETER Yes
  確認プロンプトを出さずに実行する。

.PARAMETER NoPull
  git pullを省き、手元のソースのまま再構築する。
#>

param(
    [switch]$Yes,
    [switch]$NoPull
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

$ComposeFile = Join-Path $ScriptDir 'compose.yaml'
$DataRoot = '/var/sahai'
$DbPath = 'db/sahai.sqlite3'
$BackupDir = 'backups'
# 残す世代数。無制限に増やすとディスクを圧迫する
$KeepBackups = 5

function Write-Log { param([string]$Message) Write-Host $Message }
function Die { param([string]$Message) Write-Error $Message; exit 1 }

function Invoke-Compose {
    param([string[]]$ComposeArgs)
    & docker compose -f $ComposeFile @ComposeArgs
    if ($LASTEXITCODE -ne 0) { throw "docker compose $($ComposeArgs -join ' ') failed" }
}

# DataRootはroot所有かつ700のためホストから直接読めない。コンテナ経由で操作する
function Invoke-InDataRoot {
    param([string]$Script)
    & docker run --rm -v "${DataRoot}:/data" alpine sh -c $Script
}

# ============================================================
# 0. 前提条件チェック
# ============================================================
function Step0-CheckPrerequisites {
    & docker info 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { Die 'Dockerデーモンに接続できません。' }
    if (-not (Test-Path $ComposeFile)) { Die "compose.yamlが見つかりません: $ComposeFile" }

    # 未セットアップの環境で実行しても意味がないため止める
    Invoke-InDataRoot "test -f /data/$DbPath" 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Die "セットアップがまだ完了していません($DataRoot/$DbPath が無い)。先に .\setup.ps1 を実行してください。"
    }

    if (-not $NoPull) {
        & git rev-parse --is-inside-work-tree 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Die 'gitリポジトリではありません。-NoPull を付けてください。' }
        # 未コミットの変更があるとpullが中断し、中途半端な状態になりうる
        $dirty = & git status --porcelain
        if ($dirty) { Die '未コミットの変更があります。退避するか、-NoPull を付けて実行してください。' }
    }
}

# ============================================================
# 1. ソースの更新
# ============================================================
function Step1-PullSource {
    if ($NoPull) {
        Write-Log 'git pullを省略します(-NoPull)。'
        return
    }
    $before = (& git rev-parse --short HEAD).Trim()
    Write-Log 'リポジトリを更新しています...'
    # マージコミットを作らせない。作られると次回のpullが複雑になる
    & git pull --ff-only
    if ($LASTEXITCODE -ne 0) { Die 'git pullに失敗しました。手動で解決してください。' }
    $after = (& git rev-parse --short HEAD).Trim()

    if ($before -eq $after) {
        Write-Log "  既に最新です($after)。"
    } else {
        Write-Log "  $before -> $after"
        & git --no-pager log --oneline "$before..$after" | ForEach-Object { Write-Log "    $_" }
    }
}

# ============================================================
# 2. DBのバックアップ
# ============================================================
function Step2-BackupDatabase {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    Write-Log 'DBをバックアップしています...'
    # sahai-serverを止めてからコピーする。稼働中のSQLiteをそのままコピーすると
    # 書き込み途中の状態を掴む可能性がある。traefikは止めないため、
    # 登録済みサービスへのアクセスは維持される
    & docker compose -f $ComposeFile stop sahai-server 2>&1 | Out-Null
    Invoke-InDataRoot "mkdir -p /data/$BackupDir && cp /data/$DbPath /data/$BackupDir/sahai-$stamp.sqlite3" | Out-Null
    if ($LASTEXITCODE -ne 0) { Die 'DBのバックアップに失敗しました。' }
    Write-Log "  $DataRoot/$BackupDir/sahai-$stamp.sqlite3"

    # 古い世代を削除する。新しい順に並べてKeepBackups個より後ろを消す
    $keep = $KeepBackups + 1
    Invoke-InDataRoot "ls -1t /data/$BackupDir/sahai-*.sqlite3 2>/dev/null | tail -n +$keep | xargs -r rm -f" 2>&1 | Out-Null
}

# ============================================================
# 3. イメージの取得(取得できなければビルド)
# ============================================================
function Step3-PullOrBuildImage {
    # setup.ps1と同じ方針。`up --pull always`の暗黙のフォールバックには頼らない
    # (取得失敗時にビルドへ回るかはdocker composeのバージョンによって変わる)
    Write-Log 'sahai-serverのイメージを取得しています...'
    & docker compose -f $ComposeFile pull sahai-server 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Log '  公開イメージを取得しました。'
    } else {
        Write-Log '  公開イメージを取得できませんでした。ソースからビルドします(数分かかります)...'
        Invoke-Compose @('build', 'sahai-server')
    }
}

# ============================================================
# 4. 起動
# ============================================================
function Step4-ComposeUp {
    Write-Log 'コンテナを起動しています...'
    # compose.yamlでtraefik/registryのタグが変わっていれば、ここで自動的に取得される
    Invoke-Compose @('up', '-d')
}

# ============================================================
# 5. 起動待ち
# ============================================================
function Step5-WaitForSahaiServerReady {
    Write-Log 'sahai-serverの起動を待っています...'
    $timeoutS = 120
    $elapsed = 0
    while ($true) {
        & docker compose -f $ComposeFile exec -T sahai-server curl -fsS 'http://localhost:8080/api/setup' 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { break }
        Start-Sleep -Seconds 2
        $elapsed += 2
        if ($elapsed -ge $timeoutS) {
            Write-Log ''
            Write-Log "sahai-serverが${timeoutS}秒以内に起動しませんでした。"
            Write-Log 'マイグレーションに失敗した可能性があります。ログを確認してください:'
            Write-Log '  docker compose -f compose.yaml logs sahai-server'
            Write-Log ''
            Write-Log 'DBを戻す場合は、直前のバックアップを書き戻してから再起動してください:'
            Write-Log "  docker run --rm -v ${DataRoot}:/data alpine sh -c 'ls -1t /data/$BackupDir'"
            exit 1
        }
    }
    Write-Log 'sahai-serverが起動しました。'
}

# ============================================================
# 6. 結果表示
# ============================================================
function Step6-PrintSummary {
    $rev = & git rev-parse --short HEAD 2>$null
    if ($LASTEXITCODE -ne 0) { $rev = '(git管理外)' }
    Write-Log ''
    Write-Log '====================================================='
    Write-Log '更新が完了しました。'
    Write-Log ''
    Write-Log "  版: $rev"
    Write-Log ''
    Write-Log '設定・証明書・サービスのデータは変更していません。'
    Write-Log '登録済みサービスは更新中も動き続けています。'
    Write-Log ''
    Write-Log 'ルーティングの生成規則が変わった場合に備え、気になるサービスは'
    Write-Log '再起動しておくと確実です:'
    Write-Log '  sahai service restart <サービス名>'
    Write-Log '====================================================='
}

Write-Log '差配を更新します。'
Write-Log '  対象: 土台のコンテナ(traefik / sahai-server / registry)'
Write-Log '  更新中、管理画面とレジストリは一時的に停止します。'
Write-Log '  登録済みサービスは停止しません。'
Write-Log ''

if (-not $Yes) {
    $answer = Read-Host '続行しますか? [y/N]'
    if ($answer -notin @('y', 'Y', 'yes', 'YES')) {
        Write-Log '中止しました。'
        exit 0
    }
}

Step0-CheckPrerequisites
Step1-PullSource
Step2-BackupDatabase
Step3-PullOrBuildImage
Step4-ComposeUp
Step5-WaitForSahaiServerReady
Step6-PrintSummary
