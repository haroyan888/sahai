// ServiceRegisterPageの期待される振る舞いを先に定義する(TDDのRED)。
// 現時点ではimage型の基本フローのみを対象とする。

import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ApiError } from '../api/client'
import { ServiceRegisterPage } from './ServiceRegisterPage'
import type { ApiClient } from '../api/client'
import type { ServiceDetail } from '../api/types'

function mockClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listServices: vi.fn(),
    getService: vi.fn(),
    createService: vi.fn(),
    updateService: vi.fn(),
    deleteService: vi.fn(),
    startService: vi.fn(),
    stopService: vi.fn(),
    restartService: vi.fn(),
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

describe('ServiceRegisterPage', () => {
  it('サービス名・ソース種別・イメージの入力欄を表示する', () => {
    render(<ServiceRegisterPage client={mockClient()} />)
    expect(screen.getByLabelText(/サービス名/)).toBeInTheDocument()
    expect(screen.getByLabelText(/イメージ/)).toBeInTheDocument()
  })

  it('image型で送信するとcontainers[0].nameがサービス名と一致した状態でcreateServiceを呼ぶ', async () => {
    const user = userEvent.setup()
    const created: ServiceDetail = {
      id: 1,
      name: 'myapp',
      subdomain: 'myapp.example.com',
      source_type: 'image',
      image: 'registry.sahai.example.com/myapp:latest',
      compose_content: null,
      env_vars: {},
      status: 'stopped',
      last_error: null,
      health_status: 'unknown',
      last_health_check_at: null,
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
      containers: [],
    }
    const client = mockClient({ createService: vi.fn().mockResolvedValue(created) })
    const onCreated = vi.fn()

    render(<ServiceRegisterPage client={client} onCreated={onCreated} />)

    await user.type(screen.getByLabelText(/サービス名/), 'myapp')
    await user.type(screen.getByLabelText(/イメージ/), 'registry.sahai.example.com/myapp:latest')
    await user.click(screen.getByRole('button', { name: '登録' }))

    await waitFor(() => {
      expect(client.createService).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'myapp',
          source_type: 'image',
          image: 'registry.sahai.example.com/myapp:latest',
          containers: [expect.objectContaining({ name: 'myapp' })],
        }),
      )
    })
    expect(onCreated).toHaveBeenCalledWith(created)
  })

  it('ソース種別でComposeを選ぶとイメージ欄が隠れ、compose_content欄が表示される(コンテナ名の入力は不要)', async () => {
    const user = userEvent.setup()
    render(<ServiceRegisterPage client={mockClient()} />)

    await user.click(screen.getByRole('radio', { name: 'Compose' }))

    expect(screen.queryByLabelText(/イメージ/)).not.toBeInTheDocument()
    expect(screen.getByLabelText(/compose_content/)).toBeInTheDocument()
    // バックエンドがcompose_contentをパースして全コンテナを自動登録するため、
    // コンテナ名を手入力させる欄は持たない
    expect(screen.queryByLabelText(/コンテナ名/)).not.toBeInTheDocument()
  })

  it('compose型で送信するとsource_type/compose_contentを渡してcreateServiceを呼ぶ(containersは空配列。バックエンドがcompose_contentから全コンテナを自動登録する)', async () => {
    const user = userEvent.setup()
    const created: ServiceDetail = {
      id: 2,
      name: 'webstack',
      subdomain: 'webstack.example.com',
      source_type: 'compose',
      image: null,
      compose_content: 'services:\n  web:\n    image: nginx\n  db:\n    image: mysql\n',
      env_vars: {},
      status: 'stopped',
      last_error: null,
      health_status: 'unknown',
      last_health_check_at: null,
      created_at: '2026-01-01T00:00:00.000Z',
      updated_at: '2026-01-01T00:00:00.000Z',
      containers: [],
    }
    const client = mockClient({ createService: vi.fn().mockResolvedValue(created) })
    const onCreated = vi.fn()

    render(<ServiceRegisterPage client={client} onCreated={onCreated} />)

    await user.type(screen.getByLabelText(/サービス名/), 'webstack')
    await user.click(screen.getByRole('radio', { name: 'Compose' }))
    await user.type(
      screen.getByLabelText(/compose_content/),
      'services:{Enter}  web:{Enter}    image: nginx{Enter}  db:{Enter}    image: mysql',
    )
    await user.click(screen.getByRole('button', { name: '登録' }))

    await waitFor(() => {
      expect(client.createService).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'webstack',
          source_type: 'compose',
          containers: [],
        }),
      )
    })
    expect(onCreated).toHaveBeenCalledWith(created)
  })

  it('バリデーションエラー(fields)をフィールドごとに表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      createService: vi
        .fn()
        .mockRejectedValue(
          new ApiError(400, 'VALIDATION_ERROR', '入力内容に誤りがあります', [
            { field: 'name', message: 'サービス名は英小文字で始まる必要があります' },
          ]),
        ),
    })

    render(<ServiceRegisterPage client={client} />)
    await user.type(screen.getByLabelText(/サービス名/), 'BadName')
    await user.click(screen.getByRole('button', { name: '登録' }))

    await waitFor(() => {
      expect(screen.getByText('サービス名は英小文字で始まる必要があります')).toBeInTheDocument()
    })
  })
})
