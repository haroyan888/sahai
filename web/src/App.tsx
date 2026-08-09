// アプリ全体のルーティング定義。
//
// 画面遷移方針(ユーザー確認済み):
// - ルートは一覧(/services)・新規登録(/services/new)・詳細(/services/:name)の3つのみ
// - 編集は詳細画面内のインライン編集/PortsEditModalで完結させ、新たな画面遷移を増やさない
//
// 認証: 固定Bearerトークンをこの画面(/login)で入力させlocalStorageに保存する
// (CLIの`sahai login`がconfig.tomlに保存するのと同じ役割)。トークンが無い間は
// /login以外のルートに直接アクセスしても/loginへ寄せる。

import type { ReactNode } from 'react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { LogOut, Settings as SettingsIcon } from 'lucide-react'
import { Link, Navigate, Route, Routes, useNavigate, useParams } from 'react-router-dom'
import { createApiClient } from './api/client'
import type { ApiClient } from './api/client'
import { clearStoredToken, getStoredToken, setStoredToken } from './auth/tokenStorage'
import { LoginPage } from './pages/LoginPage'
import { NotServicePage } from './pages/NotServicePage'
import { ServiceListPage } from './pages/ServiceListPage'
import { ServiceRegisterPage } from './pages/ServiceRegisterPage'
import { ServiceDetailPage } from './pages/ServiceDetailPage'
import { SettingsPage } from './pages/SettingsPage'

const API_BASE_URL: string = import.meta.env.VITE_API_BASE_URL ?? ''

function AuthShell({ children }: { children: ReactNode }) {
  return (
    <div className="app-shell">
      <main className="app-main" style={{ margin: 'auto', maxWidth: 360, width: '100%' }}>
        {children}
      </main>
    </div>
  )
}

function Layout({ children, onLogout }: { children: ReactNode; onLogout?: () => void }) {
  return (
    <div className="app-shell">
      <header className="app-header">
        <Link to="/services" className="app-header__brand">
          差配 Sahai
        </Link>
        {onLogout && (
          <div className="app-header__actions">
            <Link className="btn btn-icon" to="/settings" title="設定" aria-label="設定">
              <SettingsIcon size={16} aria-hidden="true" />
            </Link>
            <button
              className="btn btn-icon"
              type="button"
              title="ログアウト"
              aria-label="ログアウト"
              onClick={onLogout}
            >
              <LogOut size={16} aria-hidden="true" />
            </button>
          </div>
        )}
      </header>
      <main className="app-main">{children}</main>
    </div>
  )
}

function ServiceRegisterRoute({ client }: { client: ApiClient }) {
  const navigate = useNavigate()
  return (
    <ServiceRegisterPage client={client} onCreated={(detail) => navigate(`/services/${detail.name}`)} />
  )
}

function ServiceDetailRoute({ client }: { client: ApiClient }) {
  const { name } = useParams<{ name: string }>()
  const navigate = useNavigate()
  return (
    <ServiceDetailPage client={client} idOrName={name ?? ''} onDeleted={() => navigate('/services')} />
  )
}

function UnauthenticatedApp({
  onLogin,
  notice,
}: {
  onLogin: (token: string) => void
  notice?: string
}) {
  return (
    <AuthShell>
      <Routes>
        <Route path="/login" element={<LoginPage onLogin={onLogin} notice={notice} />} />
        <Route path="*" element={<Navigate to="/login" replace />} />
      </Routes>
    </AuthShell>
  )
}

function AuthenticatedApp({
  client,
  onLogout,
  onTokenChanged,
}: {
  client: ApiClient
  onLogout: () => void
  onTokenChanged: (token: string) => void
}) {
  return (
    <Layout onLogout={onLogout}>
      <Routes>
        <Route path="/services" element={<ServiceListPage client={client} />} />
        <Route path="/services/new" element={<ServiceRegisterRoute client={client} />} />
        <Route path="/services/:name" element={<ServiceDetailRoute client={client} />} />
        <Route
          path="/settings"
          element={<SettingsPage client={client} onTokenChanged={onTokenChanged} />}
        />
        <Route path="/login" element={<Navigate to="/services" replace />} />
        <Route path="*" element={<Navigate to="/services" replace />} />
      </Routes>
    </Layout>
  )
}

/// 初期設定が未完了のときの案内。初期設定はセットアップスクリプトの責務であり
/// 、Web UIからは行えないため、
/// ログイン画面を出す代わりに何をすべきかを示す。
function SetupRequired() {
  return (
    <div className="card">
      <h1>初期設定が必要です</h1>
      <p>
        サーバーの初期設定がまだ完了していません。Dockerホスト上でセットアップスクリプトを実行してください。
      </p>
      <pre>
        <code>
          ./setup.sh{'\n'}
          .\setup.ps1 (Windows)
        </code>
      </pre>
      <p className="muted">
        初期設定にはサーバーが発行するセットアップトークンが必要なため、この画面からは実行できません。
        完了後、スクリプトが表示するAPIトークンでログインしてください。
      </p>
    </div>
  )
}

function App() {
  const [token, setToken] = useState<string | null>(() => getStoredToken())
  // サーバーが未設定(api_tokenが空)の間はログインしても401になるため、
  // ログイン画面の代わりにセットアップスクリプトの実行を案内する。
  // null=判定中、判定できなかった場合は通常のログインフローへフェイルセーフする
  const [configured, setConfigured] = useState<boolean | null>(null)
  // 401で強制ログアウトした理由をログイン画面に伝える
  const [authNotice, setAuthNotice] = useState<string | undefined>(undefined)
  const navigate = useNavigate()

  useEffect(() => {
    let cancelled = false
    fetch(`${API_BASE_URL}/api/setup`)
      .then((res) => res.json() as Promise<{ configured: boolean }>)
      .then((data) => {
        if (!cancelled) setConfigured(data.configured)
      })
      .catch(() => {
        if (!cancelled) setConfigured(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  function handleLogin(newToken: string) {
    setStoredToken(newToken)
    setToken(newToken)
    setAuthNotice(undefined)
    navigate('/services')
  }

  function handleLogout() {
    clearStoredToken()
    setToken(null)
    navigate('/login')
  }

  function handleTokenChanged(newToken: string) {
    setStoredToken(newToken)
    setToken(newToken)
  }

  // APIが401を返したら、各画面が「取得に失敗しました」と表示して行き止まりに
  // なる前にログイン画面へ戻す。ポーリング中の複数リクエストが同時に401を返しても
  // 一度だけ処理すればよいので、状態の更新は冪等にしておく
  const handleUnauthorized = useCallback(() => {
    clearStoredToken()
    setToken(null)
    setAuthNotice('セッションが無効になりました。APIトークンを入力し直してください。')
    navigate('/login')
  }, [navigate])
  // useMemoの依存にコールバックを入れるとclientが作り直されポーリングが
  // リセットされるため、refを介して常に最新の関数を呼ぶ
  const unauthorizedRef = useRef(handleUnauthorized)
  unauthorizedRef.current = handleUnauthorized

  // tokenが変わらない限り同じApiClientインスタンスを保つ。JSX内で毎回
  // createApiClientを呼ぶと、ServiceListPage/ServiceDetailPageのポーリングeffectが
  // 依存配列に持つclientの参照が変わり、Appの再レンダーのたびにポーリングが
  // リセットされてしまうため
  const client = useMemo(
    () =>
      createApiClient({
        baseUrl: API_BASE_URL,
        token: token ?? '',
        onUnauthorized: () => unauthorizedRef.current(),
      }),
    [token],
  )

  return (
    <Routes>
      {/* ログイン状態に関わらず公開されるルート。Traefikが非HTTPサービス・
          未登録サブドメイン宛てのアクセスをすべてsahai-server自身へ転送してくるため、
          認証ゲートより先に判定する必要がある */}
      <Route path="/not-service" element={<NotServicePage apiBaseUrl={API_BASE_URL} />} />
      <Route
        path="*"
        element={
          configured === null ? (
            <AuthShell>
              <p className="muted">読み込み中...</p>
            </AuthShell>
          ) : !configured ? (
            <AuthShell>
              <SetupRequired />
            </AuthShell>
          ) : token ? (
            <AuthenticatedApp client={client} onLogout={handleLogout} onTokenChanged={handleTokenChanged} />
          ) : (
            <UnauthenticatedApp onLogin={handleLogin} notice={authNotice} />
          )
        }
      />
    </Routes>
  )
}

export default App
