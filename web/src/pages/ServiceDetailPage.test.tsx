// ServiceDetailPageの期待される振る舞いを先に定義する(TDDのRED)。

import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ServiceDetailPage } from './ServiceDetailPage'
import type { ApiClient } from '../api/client'
import type { ServiceDetail as ServiceDetailType } from '../api/types'

function detail(overrides: Partial<ServiceDetailType> = {}): ServiceDetailType {
  return {
    id: 1,
    name: 'myapp',
    subdomain: 'myapp.example.com',
    source_type: 'image',
    image: 'registry.sahai.example.com/myapp:latest',
    compose_content: null,
    env_vars: {},
    status: 'stopped',
    health_status: 'unknown',
    last_health_check_at: null,
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
    containers: [
      {
        id: 10,
        name: 'myapp',
        health_status: 'unknown',
        last_health_check_at: null,
        ports: [{ id: 100, container_port: 80, host_port: 20001, protocol: 'tcp', is_http: true }],
        volumes: [{ id: 200, container_path: '/data' }],
      },
    ],
    ...overrides,
  }
}

// 編集/削除は「その他の操作」メニューの配下にまとめられているため、
// クリックする前に必ずこのヘルパーでメニューを開く
async function openMenu(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'その他の操作' }))
}

function mockClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listServices: vi.fn(),
    getService: vi.fn().mockResolvedValue(detail()),
    createService: vi.fn(),
    updateService: vi.fn(),
    deleteService: vi.fn().mockResolvedValue(undefined),
    startService: vi.fn().mockResolvedValue(detail({ status: 'running' })),
    stopService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })),
    restartService: vi.fn().mockResolvedValue(detail({ status: 'running' })),
    getHealth: vi.fn().mockResolvedValue({ health_status: 'unknown', last_health_check_at: null, containers: [] }),
    getStats: vi.fn().mockResolvedValue({ containers: [] }),
    getRegistryStatus: vi.fn().mockResolvedValue({ containers: [] }),
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
    getDnsConfig: vi.fn(),
    updateDnsConfig: vi.fn(),
    getRegistryConfig: vi.fn(),
    updateRegistryConfig: vi.fn(),
    ...overrides,
  }
}

describe('ServiceDetailPage', () => {
  it('読み込み中はローディング表示をする', () => {
    const client = mockClient({ getService: vi.fn().mockReturnValue(new Promise(() => {})) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)
    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('サービス名・サブドメイン・コンテナのポート/ボリュームを表示する', async () => {
    const client = mockClient()
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await waitFor(() => {
      expect(screen.getByText('myapp')).toBeInTheDocument()
      expect(screen.getByText('myapp.example.com')).toBeInTheDocument()
      expect(screen.getByText(/20001/)).toBeInTheDocument()
      expect(screen.getByText(/\/data/)).toBeInTheDocument()
    })
    expect(client.getService).toHaveBeenCalledWith('myapp')
  })

  it('停止中は起動ボタンのみ表示する', async () => {
    const client = mockClient({ getService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    expect(await screen.findByRole('button', { name: '起動' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '停止' })).not.toBeInTheDocument()
  })

  it('稼働中は停止・再起動ボタンを表示する', async () => {
    const client = mockClient({ getService: vi.fn().mockResolvedValue(detail({ status: 'running' })) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    expect(await screen.findByRole('button', { name: '停止' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '再起動' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '起動' })).not.toBeInTheDocument()
  })

  it('起動ボタンを押すとstartServiceを呼び、結果で表示を更新する', async () => {
    const user = userEvent.setup()
    const client = mockClient({ getService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await user.click(await screen.findByRole('button', { name: '起動' }))

    expect(client.startService).toHaveBeenCalledWith('myapp')
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: '起動' })).not.toBeInTheDocument()
    })
  })

  it('起動結果にroute_warningが含まれる場合はアラートを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      getService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })),
      startService: vi
        .fn()
        .mockResolvedValue(
          detail({ status: 'running', route_warning: 'Traefikルートの反映に失敗しました: boom' }),
        ),
    })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await user.click(await screen.findByRole('button', { name: '起動' }))

    expect(await screen.findByText(/Traefikルートの反映に失敗しました/)).toBeInTheDocument()
  })

  it('route_warningが無い通常の起動結果ではアラートを表示しない', async () => {
    const user = userEvent.setup()
    const client = mockClient({ getService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await user.click(await screen.findByRole('button', { name: '起動' }))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument()
    })
    expect(screen.queryByText(/Traefikルートの反映に失敗しました/)).not.toBeInTheDocument()
  })

  it('再起動ボタンを押すと処理中の表示になりボタンが無効化され、完了後に元に戻る', async () => {
    const user = userEvent.setup()
    let resolveRestart: (value: ServiceDetailType) => void = () => {}
    const restartService = vi.fn(
      () =>
        new Promise<ServiceDetailType>((resolve) => {
          resolveRestart = resolve
        }),
    )
    const client = mockClient({
      getService: vi.fn().mockResolvedValue(detail({ status: 'running' })),
      restartService,
    })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    const restartButton = await screen.findByRole('button', { name: '再起動' })
    await user.click(restartButton)

    expect(client.restartService).toHaveBeenCalledWith('myapp')
    expect(await screen.findByRole('button', { name: /再起動中/ })).toBeDisabled()
    // 再起動中は他の操作(停止)ボタンも誤操作防止のため無効化する
    expect(screen.getByRole('button', { name: '停止' })).toBeDisabled()

    resolveRestart(detail({ status: 'running' }))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: '再起動' })).not.toBeDisabled()
    })
  })

  it('起動処理中は起動ボタンが「起動中...」表示になり無効化される', async () => {
    const user = userEvent.setup()
    let resolveStart: (value: ServiceDetailType) => void = () => {}
    const startService = vi.fn(
      () =>
        new Promise<ServiceDetailType>((resolve) => {
          resolveStart = resolve
        }),
    )
    const client = mockClient({
      getService: vi.fn().mockResolvedValue(detail({ status: 'stopped' })),
      startService,
    })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await user.click(await screen.findByRole('button', { name: '起動' }))

    expect(await screen.findByRole('button', { name: /起動中/ })).toBeDisabled()

    resolveStart(detail({ status: 'running' }))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument()
    })
  })

  it('削除ボタンは即削除せず確認モーダルを挟む', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await openMenu(user)
    await user.click(await screen.findByRole('button', { name: '削除' }))

    expect(client.deleteService).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByText(/を削除しますか/)).toBeInTheDocument()
  })

  it('削除確認後にdeleteServiceを呼ぶ(purgeVolumesはデフォルトfalse)', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await openMenu(user)
    await user.click(await screen.findByRole('button', { name: '削除' }))
    await user.click(screen.getByRole('button', { name: '削除を確定' }))

    expect(client.deleteService).toHaveBeenCalledWith('myapp', false)
  })

  it('削除確定後にonDeletedを呼ぶ(呼び出し側で一覧画面への遷移に使う)', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    const onDeleted = vi.fn()
    render(<ServiceDetailPage client={client} idOrName="myapp" onDeleted={onDeleted} />)

    await openMenu(user)
    await user.click(await screen.findByRole('button', { name: '削除' }))
    await user.click(screen.getByRole('button', { name: '削除を確定' }))

    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalled()
    })
  })

  it('削除キャンセルでモーダルを閉じ、deleteServiceを呼ばない', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await openMenu(user)
    await user.click(await screen.findByRole('button', { name: '削除' }))
    await user.click(screen.getByRole('button', { name: '閉じる' }))

    expect(client.deleteService).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const client = mockClient({ getService: vi.fn().mockRejectedValue(new Error('boom')) })
    render(<ServiceDetailPage client={client} idOrName="myapp" />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })

  describe('編集モーダル(name/image/env_vars等を1階層のモーダルで編集)', () => {
    it('「編集」ボタンでモーダルが開き、現在値が初期値として入る', async () => {
      const user = userEvent.setup()
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))

      expect(screen.getByRole('dialog')).toBeInTheDocument()
      expect(screen.getByLabelText(/サービス名/)).toHaveValue('myapp')
      expect(screen.getByLabelText(/イメージ/)).toHaveValue('registry.sahai.example.com/myapp:latest')
    })

    it('保存すると変更したフィールドを含めてupdateServiceを呼び、表示に反映する', async () => {
      const user = userEvent.setup()
      const client = mockClient({
        updateService: vi.fn().mockResolvedValue(detail({ name: 'renamed' })),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))
      const nameInput = screen.getByLabelText(/サービス名/)
      await user.clear(nameInput)
      await user.type(nameInput, 'renamed')
      await user.click(screen.getByRole('button', { name: '保存' }))

      expect(client.updateService).toHaveBeenCalledWith(
        'myapp',
        expect.objectContaining({ name: 'renamed' }),
      )
      await waitFor(() => {
        expect(screen.getByText('renamed')).toBeInTheDocument()
      })
    })

    it('キャンセルすると入力を破棄し、updateServiceを呼ばない', async () => {
      const user = userEvent.setup()
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))
      const nameInput = screen.getByLabelText(/サービス名/)
      await user.clear(nameInput)
      await user.type(nameInput, 'discarded')
      await user.click(screen.getByRole('button', { name: '閉じる' }))

      expect(client.updateService).not.toHaveBeenCalled()
      expect(screen.getByText('myapp')).toBeInTheDocument()
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  describe('ヘルス/統計情報の表示(優先度低)', () => {
    it('コンテナごとのヘルス詳細(getHealthの結果)を表示する', async () => {
      const client = mockClient({
        getHealth: vi.fn().mockResolvedValue({
          health_status: 'healthy',
          last_health_check_at: '2026-01-01T00:00:00.000Z',
          containers: [{ id: 10, name: 'myapp', health_status: 'unhealthy', last_health_check_at: null }],
        }),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await waitFor(() => {
        expect(within(screen.getByTestId('container-health-10')).getByTestId('health-badge')).toHaveAttribute(
          'data-health',
          'unhealthy',
        )
      })
      expect(client.getHealth).toHaveBeenCalledWith('myapp')
    })

    it('コンテナごとのCPU/メモリ使用量(getStatsの結果)を単位付きで表示する', async () => {
      const client = mockClient({
        getStats: vi.fn().mockResolvedValue({
          containers: [
            { id: 10, name: 'myapp', cpu_percent: 12.5, memory_usage_bytes: 1048576, memory_limit_bytes: 2097152 },
          ],
        }),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await waitFor(() => {
        expect(screen.getByTestId('container-stats-10')).toHaveTextContent('12.5%')
        expect(screen.getByTestId('container-stats-10')).toHaveTextContent('1.0 MB/2.0 MB')
      })
      expect(client.getStats).toHaveBeenCalledWith('myapp')
    })

    it('コンテナごとにレジストリへの登録有無(getRegistryStatusの結果)をイメージタグ付きで表示する', async () => {
      const client = mockClient({
        getRegistryStatus: vi.fn().mockResolvedValue({
          containers: [
            { id: 10, name: 'myapp', image_tag: 'registry.sahai.example.com/myapp:latest', image_present: false },
          ],
        }),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await waitFor(() => {
        const el = screen.getByTestId('container-registry-10')
        expect(el).toHaveTextContent('registry.sahai.example.com/myapp:latest')
        expect(el).toHaveTextContent('未登録')
      })
      expect(client.getRegistryStatus).toHaveBeenCalledWith('myapp')
    })

    it('ヘルス/統計情報の取得に失敗しても詳細画面自体はcrashしない', async () => {
      const client = mockClient({
        getHealth: vi.fn().mockRejectedValue(new Error('boom')),
        getStats: vi.fn().mockRejectedValue(new Error('boom')),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await waitFor(() => {
        expect(screen.getByText('myapp')).toBeInTheDocument()
      })
    })
  })

  describe('env_varsのインライン編集(key-valueペアの追加/削除)', () => {
    it('編集モードで既存のenv_varsがキー・バリューの入力欄として表示される', async () => {
      const user = userEvent.setup()
      const client = mockClient({
        getService: vi.fn().mockResolvedValue(detail({ env_vars: { FOO: 'bar' } })),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))

      expect(screen.getByDisplayValue('FOO')).toBeInTheDocument()
      expect(screen.getByDisplayValue('bar')).toBeInTheDocument()
    })

    it('「環境変数を追加」でその場に空の行が増える', async () => {
      const user = userEvent.setup()
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))
      expect(screen.queryAllByTestId('env-var-row')).toHaveLength(0)

      await user.click(screen.getByRole('button', { name: '環境変数を追加' }))
      expect(screen.getAllByTestId('env-var-row')).toHaveLength(1)
    })

    it('環境変数の行の削除ボタンで該当行が消える', async () => {
      const user = userEvent.setup()
      const client = mockClient({
        getService: vi.fn().mockResolvedValue(detail({ env_vars: { FOO: 'bar' } })),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))
      expect(screen.getAllByTestId('env-var-row')).toHaveLength(1)

      await user.click(screen.getByRole('button', { name: '環境変数を削除' }))
      expect(screen.queryAllByTestId('env-var-row')).toHaveLength(0)
    })

    it('保存すると空でないキーのみenv_varsとしてupdateServiceに渡される', async () => {
      const user = userEvent.setup()
      const client = mockClient({
        getService: vi.fn().mockResolvedValue(detail({ env_vars: {} })),
        updateService: vi.fn().mockResolvedValue(detail({ env_vars: { NEW_KEY: 'value1' } })),
      })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await openMenu(user)
      await user.click(await screen.findByRole('button', { name: '編集' }))
      await user.click(screen.getByRole('button', { name: '環境変数を追加' }))

      const row = screen.getByTestId('env-var-row')
      const [keyInput, valueInput] = within(row).getAllByRole('textbox')
      await user.type(keyInput, 'NEW_KEY')
      await user.type(valueInput, 'value1')

      await user.click(screen.getByRole('button', { name: '保存' }))

      expect(client.updateService).toHaveBeenCalledWith(
        'myapp',
        expect.objectContaining({ env_vars: { NEW_KEY: 'value1' } }),
      )
    })
  })

  describe('ポート/ボリューム編集(1階層のモーダル。画面遷移方針によりサブモーダルは開かない)', () => {
    it('「ポート/ボリュームを編集」ボタンでモーダルを開く', async () => {
      const user = userEvent.setup()
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await user.click(await screen.findByRole('button', { name: 'ポート/ボリュームを編集' }))

      expect(screen.getByRole('dialog')).toBeInTheDocument()
    })

    it('モーダルの保存操作でupdateServiceにcontainersを渡し、モーダルを閉じる', async () => {
      const user = userEvent.setup()
      const client = mockClient({ updateService: vi.fn().mockResolvedValue(detail()) })
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await user.click(await screen.findByRole('button', { name: 'ポート/ボリュームを編集' }))
      await user.click(screen.getByRole('button', { name: '保存' }))

      expect(client.updateService).toHaveBeenCalledWith(
        'myapp',
        expect.objectContaining({
          containers: [
            expect.objectContaining({
              name: 'myapp',
              ports: [expect.objectContaining({ container_port: 80, host_port: 20001 })],
              volumes: [{ container_path: '/data' }],
            }),
          ],
        }),
      )
      await waitFor(() => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
    })

    it('モーダルを閉じるだけならupdateServiceを呼ばない', async () => {
      const user = userEvent.setup()
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await user.click(await screen.findByRole('button', { name: 'ポート/ボリュームを編集' }))
      await user.click(screen.getByRole('button', { name: '閉じる' }))

      expect(client.updateService).not.toHaveBeenCalled()
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  describe('ポーリング', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('数秒ごとにgetServiceを再取得する', async () => {
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await vi.advanceTimersByTimeAsync(0)
      expect(client.getService).toHaveBeenCalledTimes(1)

      await vi.advanceTimersByTimeAsync(5000)
      expect(client.getService).toHaveBeenCalledTimes(2)
    })

    it('数秒ごとにgetHealth/getStatsを再取得する', async () => {
      const client = mockClient()
      render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await vi.advanceTimersByTimeAsync(0)
      expect(client.getHealth).toHaveBeenCalledTimes(1)
      expect(client.getStats).toHaveBeenCalledTimes(1)

      await vi.advanceTimersByTimeAsync(5000)
      expect(client.getHealth).toHaveBeenCalledTimes(2)
      expect(client.getStats).toHaveBeenCalledTimes(2)
    })

    it('アンマウント後はポーリングを停止する', async () => {
      const client = mockClient()
      const { unmount } = render(<ServiceDetailPage client={client} idOrName="myapp" />)

      await vi.advanceTimersByTimeAsync(0)
      expect(client.getService).toHaveBeenCalledTimes(1)
      unmount()

      await vi.advanceTimersByTimeAsync(10000)
      expect(client.getService).toHaveBeenCalledTimes(1)
    })
  })
})
