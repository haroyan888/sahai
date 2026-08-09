// LoginPageの期待される振る舞いを先に定義する(TDDのRED)。
// CLIの`sahai login`に相当する、Web UI側でのAPIトークン入力画面。

import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { LoginPage } from './LoginPage'

describe('LoginPage', () => {
  it('APIトークンの入力欄とログインボタンを表示する', () => {
    render(<LoginPage onLogin={vi.fn()} />)
    expect(screen.getByLabelText(/APIトークン/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'ログイン' })).toBeInTheDocument()
  })

  it('入力したトークンでonLoginを呼ぶ', async () => {
    const user = userEvent.setup()
    const onLogin = vi.fn()
    render(<LoginPage onLogin={onLogin} />)

    await user.type(screen.getByLabelText(/APIトークン/), 'my-secret-token')
    await user.click(screen.getByRole('button', { name: 'ログイン' }))

    expect(onLogin).toHaveBeenCalledWith('my-secret-token')
  })

  it('未入力のまま送信してもonLoginを呼ばない', async () => {
    const user = userEvent.setup()
    const onLogin = vi.fn()
    render(<LoginPage onLogin={onLogin} />)

    await user.click(screen.getByRole('button', { name: 'ログイン' }))

    expect(onLogin).not.toHaveBeenCalled()
  })

  it('noticeが渡された場合は再ログインを促すメッセージを表示する', () => {
    render(<LoginPage onLogin={vi.fn()} notice="セッションが無効になりました。APIトークンを入力し直してください。" />)
    expect(screen.getByRole('alert')).toHaveTextContent('セッションが無効になりました')
  })

  it('noticeが無い場合はメッセージを表示しない', () => {
    render(<LoginPage onLogin={vi.fn()} />)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })
})
