// RegistryConfigSectionの期待される振る舞いを先に定義する(TDDのRED)。
// docker loginは同期的にすぐ終わる軽い処理で接続断も起きないため、DnsConfigSectionの
// ような再接続ポーリングは無く、保存に成功したら即座にsaved/login_warningが確定する。

import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { RegistryConfigSection } from './RegistryConfigSection'
import { ApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { RegistryConfig } from '../api/types'

function registryConfig(overrides: Partial<RegistryConfig> = {}): RegistryConfig {
  return {
    registry_url: 'registry.sahai.example.com',
    registry_username: 'reguser',
    registry_password: 'regpass',
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
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
    getDnsConfig: vi.fn(),
    updateDnsConfig: vi.fn(),
    getRegistryConfig: vi.fn().mockResolvedValue(registryConfig()),
    updateRegistryConfig: vi.fn().mockResolvedValue(registryConfig()),
    ...overrides,
  }
}

describe('RegistryConfigSection', () => {
  it('読み込み中はローディング表示をする', () => {
    const client = mockClient({ getRegistryConfig: vi.fn().mockReturnValue(new Promise(() => {})) })
    render(<RegistryConfigSection client={client} />)
    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const client = mockClient({ getRegistryConfig: vi.fn().mockRejectedValue(new Error('boom')) })
    render(<RegistryConfigSection client={client} />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })

  it('取得したレジストリ設定をフォームの初期値として表示する', async () => {
    const client = mockClient()
    render(<RegistryConfigSection client={client} />)

    expect(await screen.findByLabelText(/レジストリURL/)).toHaveValue('registry.sahai.example.com')
    expect(screen.getByLabelText(/ユーザー名/)).toHaveValue('reguser')
    expect(screen.getByLabelText(/パスワード/)).toHaveValue('regpass')
  })

  it('パスワード欄はtype="password"である', async () => {
    const client = mockClient()
    render(<RegistryConfigSection client={client} />)

    const passwordInput = await screen.findByLabelText(/パスワード/)
    expect(passwordInput).toHaveAttribute('type', 'password')
  })

  it('保存ボタンでupdateRegistryConfigを呼び、成功したら保存完了メッセージを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<RegistryConfigSection client={client} />)

    const urlInput = await screen.findByLabelText(/レジストリURL/)
    await user.clear(urlInput)
    await user.type(urlInput, 'registry.new.example.com')
    await user.click(screen.getByRole('button', { name: 'レジストリ設定を保存' }))

    await waitFor(() => {
      expect(client.updateRegistryConfig).toHaveBeenCalledWith(
        expect.objectContaining({ registry_url: 'registry.new.example.com' }),
      )
      expect(screen.getByText('保存しました')).toBeInTheDocument()
    })
  })

  it('保存レスポンスにlogin_warningが含まれる場合は警告メッセージを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      updateRegistryConfig: vi.fn().mockResolvedValue(
        registryConfig({ login_warning: 'レジストリへのログインに失敗しました: unauthorized' }),
      ),
    })
    render(<RegistryConfigSection client={client} />)

    await screen.findByLabelText(/レジストリURL/)
    await user.click(screen.getByRole('button', { name: 'レジストリ設定を保存' }))

    await waitFor(() => {
      expect(screen.getByText(/レジストリへのログインに失敗しました: unauthorized/)).toBeInTheDocument()
      expect(screen.getByText('保存しました')).toBeInTheDocument()
    })
  })

  it('login_warningを含まないレスポンスなら警告は表示されない', async () => {
    const user = userEvent.setup()
    const client = mockClient()
    render(<RegistryConfigSection client={client} />)

    await screen.findByLabelText(/レジストリURL/)
    await user.click(screen.getByRole('button', { name: 'レジストリ設定を保存' }))

    await waitFor(() => {
      expect(screen.getByText('保存しました')).toBeInTheDocument()
    })
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('バリデーションエラー時はフィールドごとのエラーメッセージを表示する', async () => {
    const user = userEvent.setup()
    const client = mockClient({
      updateRegistryConfig: vi
        .fn()
        .mockRejectedValue(
          new ApiError(400, 'VALIDATION_ERROR', '入力内容に誤りがあります', [
            { field: 'registry_password', message: 'ユーザー名とパスワードは両方入力するか、両方空にしてください' },
          ]),
        ),
    })
    render(<RegistryConfigSection client={client} />)

    await screen.findByLabelText(/レジストリURL/)
    await user.click(screen.getByRole('button', { name: 'レジストリ設定を保存' }))

    await waitFor(() => {
      expect(
        screen.getByText('ユーザー名とパスワードは両方入力するか、両方空にしてください'),
      ).toBeInTheDocument()
    })
  })
})
