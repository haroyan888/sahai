// DnsConfigSectionの期待される振る舞いを先に定義する(TDDのRED)。
// 保存するとバックエンド側でTraefikコンテナが再作成されるため、保存直後は
// 「再接続中」の状態を経由し、getDnsConfigが再び成功したら完了とみなす設計。

import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DnsConfigSection } from './DnsConfigSection'
import { ApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { DnsConfig } from '../api/types'

function dnsConfig(overrides: Partial<DnsConfig> = {}): DnsConfig {
  return {
    dns_provider: 'cloudflare',
    acme_email: 'admin@example.com',
    credentials: [{ key: 'CF_DNS_API_TOKEN', value: 'abc123' }],
    ...overrides,
  }
}

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
    getDnsConfig: vi.fn().mockResolvedValue(dnsConfig()),
    updateDnsConfig: vi.fn().mockResolvedValue(dnsConfig()),
    getRegistryConfig: vi.fn(),
    updateRegistryConfig: vi.fn(),
    ...overrides,
  }
}

afterEach(() => {
  vi.useRealTimers()
})

describe('DnsConfigSection', () => {
  it('読み込み中はローディング表示をする', () => {
    const client = mockClient({ getDnsConfig: vi.fn().mockReturnValue(new Promise(() => {})) })
    render(<DnsConfigSection client={client} />)
    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const client = mockClient({ getDnsConfig: vi.fn().mockRejectedValue(new Error('boom')) })
    render(<DnsConfigSection client={client} />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })

  it('取得したDNS設定をフォームの初期値として表示する', async () => {
    const client = mockClient()
    render(<DnsConfigSection client={client} />)

    expect(await screen.findByLabelText(/DNSプロバイダ/)).toHaveValue('cloudflare')
    expect(screen.getByLabelText(/ACME通知先メールアドレス/)).toHaveValue('admin@example.com')
    expect(screen.getByLabelText('認証情報キー')).toHaveValue('CF_DNS_API_TOKEN')
    expect(screen.getByLabelText('認証情報の値')).toHaveValue('abc123')
  })

  it('保存すると一時的に接続が切れる旨を案内する', async () => {
    const client = mockClient()
    render(<DnsConfigSection client={client} />)
    await screen.findByLabelText(/DNSプロバイダ/)
    expect(screen.getByText(/接続が一時的に切れ/)).toBeInTheDocument()
  })

  it('認証情報の追加・削除ができる', async () => {
    const user = userEvent.setup()
    const client = mockClient({ getDnsConfig: vi.fn().mockResolvedValue(dnsConfig({ credentials: [] })) })
    render(<DnsConfigSection client={client} />)
    await screen.findByLabelText(/DNSプロバイダ/)

    expect(screen.queryAllByTestId('dns-credential-row')).toHaveLength(0)
    await user.click(screen.getByRole('button', { name: '認証情報を追加' }))
    expect(screen.getAllByTestId('dns-credential-row')).toHaveLength(1)

    await user.type(screen.getByLabelText('認証情報キー'), 'CF_DNS_API_TOKEN')
    await user.type(screen.getByLabelText('認証情報の値'), 'secret-token')

    await user.click(screen.getByRole('button', { name: '認証情報を削除' }))
    expect(screen.queryAllByTestId('dns-credential-row')).toHaveLength(0)
  })

  it('保存ボタンでupdateDnsConfigを呼ぶ', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<DnsConfigSection client={client} />)

    const providerInput = await screen.findByLabelText(/DNSプロバイダ/)
    await user.clear(providerInput)
    await user.type(providerInput, 'route53')
    await user.click(screen.getByRole('button', { name: 'DNS設定を保存' }))

    await waitFor(() => {
      expect(client.updateDnsConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          dns_provider: 'route53',
          acme_email: 'admin@example.com',
          credentials: [{ key: 'CF_DNS_API_TOKEN', value: 'abc123' }],
        }),
      )
    })
  })

  it('バリデーションエラー時はTraefik再作成を待たずにフィールドエラーを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      updateDnsConfig: vi
        .fn()
        .mockRejectedValue(
          new ApiError(400, 'VALIDATION_ERROR', '入力内容に誤りがあります', [
            { field: 'dns_provider', message: 'DNSプロバイダを入力してください' },
          ]),
        ),
    })
    render(<DnsConfigSection client={client} />)

    await screen.findByLabelText(/DNSプロバイダ/)
    await user.click(screen.getByRole('button', { name: 'DNS設定を保存' }))

    await waitFor(() => {
      expect(screen.getByText('DNSプロバイダを入力してください')).toBeInTheDocument()
    })
    expect(screen.queryByText(/再接続を待っています/)).not.toBeInTheDocument()
  })

  it('保存後は再接続中のメッセージを表示し、再接続を確認できたら完了メッセージに変わる', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const user = userEvent.setup({ delay: null })
    const client = mockClient({
      getDnsConfig: vi
        .fn()
        .mockResolvedValueOnce(dnsConfig()) // 初回読み込み
        .mockRejectedValueOnce(new Error('network error')) // Traefik再作成中でまだ繋がらない
        .mockResolvedValueOnce(dnsConfig()), // 再接続できた
    })
    render(<DnsConfigSection client={client} />)

    await screen.findByLabelText(/DNSプロバイダ/)
    await user.click(screen.getByRole('button', { name: 'DNS設定を保存' }))

    expect(await screen.findByText(/再接続を待っています/)).toBeInTheDocument()

    await vi.advanceTimersByTimeAsync(2000)
    await vi.advanceTimersByTimeAsync(2000)

    expect(await screen.findByText(/再接続を確認しました/)).toBeInTheDocument()
  })

  it('再接続中は経過秒数を表示し、固まっていないことが分かるようにする', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const user = userEvent.setup({ delay: null })
    const client = mockClient({
      getDnsConfig: vi
        .fn()
        .mockResolvedValueOnce(dnsConfig()) // 初回読み込み
        .mockRejectedValueOnce(new Error('network error'))
        .mockRejectedValueOnce(new Error('network error'))
        .mockResolvedValueOnce(dnsConfig()),
    })
    render(<DnsConfigSection client={client} />)

    await screen.findByLabelText(/DNSプロバイダ/)
    await user.click(screen.getByRole('button', { name: 'DNS設定を保存' }))

    expect(await screen.findByText(/経過: 0秒/)).toBeInTheDocument()

    await vi.advanceTimersByTimeAsync(2000)
    expect(await screen.findByText(/経過: 2秒/)).toBeInTheDocument()

    await vi.advanceTimersByTimeAsync(2000)
    expect(await screen.findByText(/経過: 4秒/)).toBeInTheDocument()
  })

  it('保存自体がネットワークエラーで失敗しても再接続待ちとして扱う', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const user = userEvent.setup({ delay: null })
    const client = mockClient({
      updateDnsConfig: vi.fn().mockRejectedValue(new TypeError('Failed to fetch')),
      getDnsConfig: vi.fn().mockResolvedValueOnce(dnsConfig()).mockResolvedValueOnce(dnsConfig()),
    })
    render(<DnsConfigSection client={client} />)

    await screen.findByLabelText(/DNSプロバイダ/)
    await user.click(screen.getByRole('button', { name: 'DNS設定を保存' }))

    expect(await screen.findByText(/再接続を待っています/)).toBeInTheDocument()

    await vi.advanceTimersByTimeAsync(2000)

    expect(await screen.findByText(/再接続を確認しました/)).toBeInTheDocument()
  })
})
