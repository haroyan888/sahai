// App.tsxのルーティング+認証ゲートを検証する(TDDのRED)。
// 各ページコンポーネント自体の振る舞いはそれぞれのテストで検証済みなので、
// ここではモックに差し替えてルーティング・画面遷移・認証状態の切り替えのみを対象とする。

import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { getStoredToken, setStoredToken } from './auth/tokenStorage'
import type { ServiceDetail } from './api/types'

const onLoginRef: { current?: (token: string) => void } = {}
const onCreatedRef: { current?: (detail: ServiceDetail) => void } = {}
const onDeletedRef: { current?: () => void } = {}

vi.mock('./pages/LoginPage', () => ({
  LoginPage: (props: { onLogin: (token: string) => void }) => {
    onLoginRef.current = props.onLogin
    return <div>LoginPageStub</div>
  },
}))

vi.mock('./pages/NotServicePage', () => ({
  NotServicePage: () => <div>NotServicePageStub</div>,
}))

vi.mock('./pages/ServiceListPage', () => ({
  ServiceListPage: () => <div>ServiceListPageStub</div>,
}))

vi.mock('./pages/ServiceRegisterPage', () => ({
  ServiceRegisterPage: (props: { onCreated?: (detail: ServiceDetail) => void }) => {
    onCreatedRef.current = props.onCreated
    return <div>ServiceRegisterPageStub</div>
  },
}))

vi.mock('./pages/ServiceDetailPage', () => ({
  ServiceDetailPage: (props: { idOrName: string; onDeleted?: () => void }) => {
    onDeletedRef.current = props.onDeleted
    return <div>ServiceDetailPageStub:{props.idOrName}</div>
  },
}))

function stubSetupStatus(configured: boolean) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ configured }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    ),
  )
}

async function renderAt(path: string) {
  const result = render(
    <MemoryRouter initialEntries={[path]}>
      <App />
    </MemoryRouter>,
  )
  // 起動時のGET /api/setup(初期設定要否の判定)を待つ
  await act(async () => {})
  return result
}

function sampleDetail(overrides: Partial<ServiceDetail> = {}): ServiceDetail {
  return {
    id: 1,
    name: 'created-app',
    subdomain: 'created-app.example.com',
    source_type: 'image',
    image: 'x:latest',
    compose_content: null,
    env_vars: {},
    status: 'stopped',
    last_error: null,
    health_status: 'unknown',
    last_health_check_at: null,
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
    containers: [],
    ...overrides,
  }
}

beforeEach(() => {
  localStorage.clear()
  // 既存デプロイ(設定済み)を前提にした従来どおりのログインフローを既定にする。
  // 未設定フローを検証するテストは個別にstubSetupStatus(false)で上書きする
  stubSetupStatus(true)
})

describe('App(認証状態に関わらず公開されるルート)', () => {
  it('トークンが無くても/not-serviceは表示できる', async () => {
    await renderAt('/not-service')
    expect(screen.getByText('NotServicePageStub')).toBeInTheDocument()
  })

  it('トークンがあっても/not-serviceは(ログイン画面や一覧へリダイレクトされず)表示できる', async () => {
    setStoredToken('preset-token')
    await renderAt('/not-service')
    expect(screen.getByText('NotServicePageStub')).toBeInTheDocument()
  })
})

describe('App(初期セットアップ未完了。サーバーがまだ未設定)', () => {
  it('未設定の場合、ログイン画面ではなくセットアップスクリプトの実行を案内する', async () => {
    stubSetupStatus(false)
    await renderAt('/services')
    expect(screen.getByText('初期設定が必要です')).toBeInTheDocument()
    expect(screen.queryByText('LoginPageStub')).not.toBeInTheDocument()
  })

  it('トークンが既にlocalStorageにあっても未設定なら案内が優先される', async () => {
    stubSetupStatus(false)
    setStoredToken('stale-token')
    await renderAt('/services')
    expect(screen.getByText('初期設定が必要です')).toBeInTheDocument()
    expect(screen.queryByText('ServiceListPageStub')).not.toBeInTheDocument()
  })

  it('設定要否を判定できない場合は通常のログインフローにフェイルセーフする', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('network error')))
    await renderAt('/services')
    expect(screen.getByText('LoginPageStub')).toBeInTheDocument()
  })
})

describe('App(未ログイン)', () => {
  it('トークンが無い場合、/servicesへのアクセスもログイン画面になる', async () => {
    await renderAt('/services')
    expect(screen.getByText('LoginPageStub')).toBeInTheDocument()
  })

  it('トークンが無い場合、/services/newへのアクセスもログイン画面になる', async () => {
    await renderAt('/services/new')
    expect(screen.getByText('LoginPageStub')).toBeInTheDocument()
  })

  it('ログイン成功でトークンを保存し一覧画面へ遷移する', async () => {
    await renderAt('/login')

    act(() => {
      onLoginRef.current?.('my-token')
    })

    expect(getStoredToken()).toBe('my-token')
    expect(screen.getByText('ServiceListPageStub')).toBeInTheDocument()
  })
})

describe('App(ログイン済み)', () => {
  beforeEach(() => {
    setStoredToken('preset-token')
  })

  it('/services で一覧画面を表示する', async () => {
    await renderAt('/services')
    expect(screen.getByText('ServiceListPageStub')).toBeInTheDocument()
  })

  it('/services/new で新規登録画面を表示する', async () => {
    await renderAt('/services/new')
    expect(screen.getByText('ServiceRegisterPageStub')).toBeInTheDocument()
  })

  it('/services/:name で詳細画面を表示する', async () => {
    await renderAt('/services/myapp')
    expect(screen.getByText('ServiceDetailPageStub:myapp')).toBeInTheDocument()
  })

  it('未定義パスは/servicesへリダイレクトする', async () => {
    await renderAt('/unknown')
    expect(screen.getByText('ServiceListPageStub')).toBeInTheDocument()
  })

  it('/loginにアクセスしても一覧画面へリダイレクトする', async () => {
    await renderAt('/login')
    expect(screen.getByText('ServiceListPageStub')).toBeInTheDocument()
  })

  it('新規登録画面のonCreatedは作成されたサービスの詳細画面へ遷移させる', async () => {
    await renderAt('/services/new')

    act(() => {
      onCreatedRef.current?.(sampleDetail())
    })

    expect(screen.getByText('ServiceDetailPageStub:created-app')).toBeInTheDocument()
  })

  it('詳細画面のonDeletedは一覧画面へ遷移させる', async () => {
    await renderAt('/services/myapp')

    act(() => {
      onDeletedRef.current?.()
    })

    expect(screen.getByText('ServiceListPageStub')).toBeInTheDocument()
  })

  it('ヘッダーのログアウトボタンでトークンを削除しログイン画面に戻る', async () => {
    const user = userEvent.setup()
    await renderAt('/services')

    await user.click(screen.getByRole('button', { name: 'ログアウト' }))

    expect(getStoredToken()).toBeNull()
    expect(screen.getByText('LoginPageStub')).toBeInTheDocument()
  })
})
