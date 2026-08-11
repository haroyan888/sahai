// SettingsPageの期待される振る舞いを先に定義する(TDDのRED)。

import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { SettingsPage } from './SettingsPage'
import { ApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { DnsConfig, RegistryConfig, Settings } from '../api/types'

function settings(overrides: Partial<Settings> = {}): Settings {
  return {
    domain: 'example.com',
    https_redirect: true,
    api_token: 'current-token',
    ...overrides,
  }
}

function dnsConfig(overrides: Partial<DnsConfig> = {}): DnsConfig {
  return {
    dns_provider: 'cloudflare',
    acme_email: '',
    credentials: [],
    ...overrides,
  }
}

function registryConfig(overrides: Partial<RegistryConfig> = {}): RegistryConfig {
  return {
    registry_url: 'registry.sahai.example.com',
    registry_username: null,
    registry_password: null,
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
    getHealth: vi.fn(),
    getStats: vi.fn(),
    getRegistryStatus: vi.fn(),
    streamLogs: vi.fn().mockResolvedValue(undefined),
    getSettings: vi.fn().mockResolvedValue(settings()),
    updateSettings: vi.fn().mockImplementation((s) => Promise.resolve(s)),
    getDnsConfig: vi.fn().mockResolvedValue(dnsConfig()),
    updateDnsConfig: vi.fn().mockImplementation((c) => Promise.resolve(c)),
    getRegistryConfig: vi.fn().mockResolvedValue(registryConfig()),
    updateRegistryConfig: vi.fn().mockImplementation((c) => Promise.resolve(c)),
    ...overrides,
  }
}

describe('SettingsPage', () => {
  it('読み込み中はローディング表示をする', () => {
    const client = mockClient({ getSettings: vi.fn().mockReturnValue(new Promise(() => {})) })
    render(<SettingsPage client={client} />)
    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('取得した設定値をフォームの初期値として表示する', async () => {
    const client = mockClient({ getSettings: vi.fn().mockResolvedValue(settings({ domain: 'example.com' })) })
    render(<SettingsPage client={client} />)

    expect(await screen.findByLabelText(/ドメイン/)).toHaveValue('example.com')
    expect(screen.getByLabelText(/APIトークン/)).toHaveValue('current-token')
    expect(screen.getByLabelText(/HTTPSへリダイレクト/)).toBeChecked()
  })

  it('基本設定フォームにレジストリURL欄が無い', async () => {
    const client = mockClient()
    render(<SettingsPage client={client} />)

    await screen.findByLabelText(/ドメイン/)
    const basicForm = screen.getByTestId('basic-settings-form')
    expect(within(basicForm).queryByLabelText(/レジストリURL/)).not.toBeInTheDocument()
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const client = mockClient({ getSettings: vi.fn().mockRejectedValue(new Error('boom')) })
    render(<SettingsPage client={client} />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })

  it('保存ボタンでupdateSettingsを呼び、成功したら保存完了メッセージを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<SettingsPage client={client} />)

    const tokenInput = await screen.findByLabelText(/APIトークン/)
    await user.clear(tokenInput)
    await user.type(tokenInput, 'another-token')
    await user.click(screen.getByRole('button', { name: '基本設定を保存' }))

    await waitFor(() => {
      expect(client.updateSettings).toHaveBeenCalledWith(
        expect.objectContaining({ api_token: 'another-token' }),
      )
      expect(screen.getByText(/保存しました/)).toBeInTheDocument()
    })
  })

  it('ドメインを変更して保存すると、新しいURLへ移動する案内を表示し通常の保存完了メッセージは出さない', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<SettingsPage client={client} />)

    const domainInput = await screen.findByLabelText(/ドメイン/)
    await user.clear(domainInput)
    await user.type(domainInput, 'newdomain.com')
    await user.click(screen.getByRole('button', { name: '基本設定を保存' }))

    await waitFor(() => {
      expect(screen.getByText(/新しいURL/)).toBeInTheDocument()
      expect(screen.getByRole('link', { name: /sahai\.newdomain\.com/ })).toHaveAttribute(
        'href',
        'https://sahai.newdomain.com/',
      )
    })
    expect(screen.queryByText('保存しました')).not.toBeInTheDocument()
  })

  it('APIトークンを変更して保存すると、新しいトークンでonTokenChangedを呼ぶ', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    const onTokenChanged = vi.fn()
    render(<SettingsPage client={client} onTokenChanged={onTokenChanged} />)

    const tokenInput = await screen.findByLabelText(/APIトークン/)
    await user.clear(tokenInput)
    await user.type(tokenInput, 'brand-new-token')
    await user.click(screen.getByRole('button', { name: '基本設定を保存' }))

    await waitFor(() => {
      expect(onTokenChanged).toHaveBeenCalledWith('brand-new-token')
    })
  })

  it('DNS/証明書設定セクションを表示する', async () => {
    const client = mockClient()
    render(<SettingsPage client={client} />)

    await screen.findByLabelText(/ドメイン/)
    expect(await screen.findByRole('heading', { name: 'DNS/証明書設定' })).toBeInTheDocument()
    expect(client.getDnsConfig).toHaveBeenCalled()
  })

  it('レジストリ設定セクションを表示する', async () => {
    const client = mockClient()
    render(<SettingsPage client={client} />)

    await screen.findByLabelText(/ドメイン/)
    expect(await screen.findByRole('heading', { name: /レジストリ設定/ })).toBeInTheDocument()
    expect(client.getRegistryConfig).toHaveBeenCalled()
  })

  it('バリデーションエラー時はフィールドごとのエラーメッセージを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      updateSettings: vi
        .fn()
        .mockRejectedValue(new ApiError(400, 'VALIDATION_ERROR', '入力内容に誤りがあります', [
          { field: 'domain', message: 'ドメインを入力してください' },
        ])),
    })
    render(<SettingsPage client={client} />)

    await screen.findByLabelText(/ドメイン/)
    await user.click(screen.getByRole('button', { name: '基本設定を保存' }))

    await waitFor(() => {
      expect(screen.getByText('ドメインを入力してください')).toBeInTheDocument()
    })
  })
})
