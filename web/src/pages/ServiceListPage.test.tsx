// ServiceListPageの期待される振る舞いを先に定義する(TDDのRED)。

import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ServiceListPage } from './ServiceListPage'
import type { ApiClient } from '../api/client'
import type { Service, ServiceDetail } from '../api/types'

function service(overrides: Partial<Service> = {}): Service {
  return {
    id: 1,
    name: 'myapp',
    subdomain: 'myapp.example.com',
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
    ...overrides,
  }
}

function mockClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listServices: vi.fn().mockResolvedValue([]),
    getService: vi.fn(),
    createService: vi.fn(),
    updateService: vi.fn(),
    deleteService: vi.fn(),
    startService: vi.fn(),
    stopService: vi.fn(),
    restartService: vi.fn(),
    getHealth: vi.fn(),
    getStats: vi.fn(),
    getRegistryStatus: vi.fn(),
    streamLogs: vi.fn().mockResolvedValue(undefined),
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
    getDnsConfig: vi.fn(),
    updateDnsConfig: vi.fn(),
    getRegistryConfig: vi.fn(),
    updateRegistryConfig: vi.fn(),
    ...overrides,
  }
}

function renderWithRouter(ui: React.ReactElement) {
  return render(<MemoryRouter>{ui}</MemoryRouter>)
}

describe('ServiceListPage', () => {
  it('読み込み中はローディング表示をする', () => {
    const client = mockClient({
      listServices: vi.fn().mockReturnValue(new Promise(() => {})), // 永遠に未解決
    })
    renderWithRouter(<ServiceListPage client={client} />)
    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('取得したサービス一覧を名前付きで表示する', async () => {
    const client = mockClient({
      listServices: vi.fn().mockResolvedValue([service({ name: 'myapp' }), service({ id: 2, name: 'webstack' })]),
    })
    renderWithRouter(<ServiceListPage client={client} />)

    await waitFor(() => {
      expect(screen.getByText('myapp')).toBeInTheDocument()
      expect(screen.getByText('webstack')).toBeInTheDocument()
    })
  })

  it('各サービスの詳細ページへのリンクを持つ', async () => {
    const client = mockClient({
      listServices: vi.fn().mockResolvedValue([service({ name: 'myapp' })]),
    })
    renderWithRouter(<ServiceListPage client={client} />)

    const link = await screen.findByRole('link', { name: /myapp/ })
    expect(link).toHaveAttribute('href', '/services/myapp')
  })

  it('稼働中サービスはヘルスをドットバッジで表示する(操作アイコンで稼働状態が分かるためステータスの文言バッジは出さない)', async () => {
    const client = mockClient({
      listServices: vi.fn().mockResolvedValue([
        service({ name: 'myapp', status: 'running', health_status: 'healthy' }),
      ]),
    })
    renderWithRouter(<ServiceListPage client={client} />)

    await waitFor(() => {
      expect(screen.queryByTestId('status-badge')).not.toBeInTheDocument()
      expect(screen.getByTestId('health-badge')).toHaveAttribute('data-health', 'healthy')
      expect(screen.getByTestId('health-badge')).toHaveTextContent('')
    })
  })

  it('停止中サービスはhealth_statusに関わらずヘルスドットを表示しない', async () => {
    const client = mockClient({
      listServices: vi.fn().mockResolvedValue([
        service({ name: 'myapp', status: 'stopped', health_status: 'unhealthy' }),
      ]),
    })
    renderWithRouter(<ServiceListPage client={client} />)

    await waitFor(() => {
      expect(screen.getByText('myapp')).toBeInTheDocument()
    })
    expect(screen.queryByTestId('health-badge')).not.toBeInTheDocument()
  })

  it('サービスが1件もない場合は案内文を表示する', async () => {
    const client = mockClient({ listServices: vi.fn().mockResolvedValue([]) })
    renderWithRouter(<ServiceListPage client={client} />)

    await waitFor(() => {
      expect(screen.getByText(/登録されているサービスがありません/)).toBeInTheDocument()
    })
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const client = mockClient({
      listServices: vi.fn().mockRejectedValue(new Error('network error')),
    })
    renderWithRouter(<ServiceListPage client={client} />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })

  it('新規登録画面へのリンクを表示する', async () => {
    const client = mockClient()
    renderWithRouter(<ServiceListPage client={client} />)

    const link = await screen.findByRole('link', { name: /新規登録/ })
    expect(link).toHaveAttribute('href', '/services/new')
  })

  describe('一覧からの起動/停止/再起動(詳細画面への遷移なしで操作できる)', () => {
    it('停止中サービスは起動ボタンのみ表示する', async () => {
      const client = mockClient({
        listServices: vi.fn().mockResolvedValue([service({ name: 'myapp', status: 'stopped' })]),
      })
      renderWithRouter(<ServiceListPage client={client} />)

      const item = (await screen.findByText('myapp')).closest('li')!
      expect(within(item).getByRole('button', { name: '起動' })).toBeInTheDocument()
      expect(within(item).queryByRole('button', { name: '停止' })).not.toBeInTheDocument()
      expect(within(item).queryByRole('button', { name: '再起動' })).not.toBeInTheDocument()
    })

    it('稼働中サービスは停止・再起動ボタンを表示する', async () => {
      const client = mockClient({
        listServices: vi.fn().mockResolvedValue([service({ name: 'myapp', status: 'running' })]),
      })
      renderWithRouter(<ServiceListPage client={client} />)

      const item = (await screen.findByText('myapp')).closest('li')!
      expect(within(item).getByRole('button', { name: '停止' })).toBeInTheDocument()
      expect(within(item).getByRole('button', { name: '再起動' })).toBeInTheDocument()
      expect(within(item).queryByRole('button', { name: '起動' })).not.toBeInTheDocument()
    })

    it('起動ボタンを押すとstartServiceを呼び、処理中は無効化・完了後に一覧を再取得する', async () => {
      const user = userEvent.setup()
      let resolveStart: () => void = () => {}
      const startService = vi.fn(
        () =>
          new Promise<ServiceDetail>((resolve) => {
            resolveStart = () => resolve({ ...service({ name: 'myapp', status: 'running' }), containers: [] })
          }),
      )
      const listServices = vi
        .fn()
        .mockResolvedValueOnce([service({ name: 'myapp', status: 'stopped' })])
        .mockResolvedValueOnce([service({ name: 'myapp', status: 'running' })])
      const client = mockClient({ listServices, startService })
      renderWithRouter(<ServiceListPage client={client} />)

      const item = (await screen.findByText('myapp')).closest('li')!
      const startButton = within(item).getByRole('button', { name: '起動' })
      await user.click(startButton)

      expect(client.startService).toHaveBeenCalledWith('myapp')
      expect(within(item).getByRole('button', { name: /起動中/ })).toBeDisabled()

      resolveStart()

      await waitFor(() => {
        expect(client.listServices).toHaveBeenCalledTimes(2)
      })
    })

  })

  describe('ポーリング', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('数秒ごとに一覧を再取得する', async () => {
      const client = mockClient({
        listServices: vi.fn().mockResolvedValue([service({ name: 'myapp' })]),
      })
      renderWithRouter(<ServiceListPage client={client} />)

      await vi.advanceTimersByTimeAsync(0)
      expect(client.listServices).toHaveBeenCalledTimes(1)

      await vi.advanceTimersByTimeAsync(5000)
      expect(client.listServices).toHaveBeenCalledTimes(2)

      await vi.advanceTimersByTimeAsync(5000)
      expect(client.listServices).toHaveBeenCalledTimes(3)
    })

    it('アンマウント後はポーリングを停止する', async () => {
      const client = mockClient({
        listServices: vi.fn().mockResolvedValue([service({ name: 'myapp' })]),
      })
      const { unmount } = renderWithRouter(<ServiceListPage client={client} />)

      await vi.advanceTimersByTimeAsync(0)
      expect(client.listServices).toHaveBeenCalledTimes(1)
      unmount()

      await vi.advanceTimersByTimeAsync(10000)
      expect(client.listServices).toHaveBeenCalledTimes(1)
    })
  })
})
