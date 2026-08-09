// ログイン画面(CLIの`sahai login`に相当)。ルート: /login。
// Control plane APIへの固定Bearerトークンを入力させ、呼び出し側(App)に渡すだけの
// controlled component。トークンの永続化自体は行わない。

import { useState } from 'react'

export interface LoginPageProps {
  onLogin: (token: string) => void
  /** 401で強制ログアウトされた直後など、再ログインを促す理由を表示する。 */
  notice?: string
}

export function LoginPage({ onLogin, notice }: LoginPageProps) {
  const [token, setToken] = useState('')

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    const trimmed = token.trim()
    if (trimmed === '') return
    onLogin(trimmed)
  }

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h1>差配 ログイン</h1>
      {notice && (
        <p className="alert" role="alert">
          {notice}
        </p>
      )}
      <p className="muted">Control PlaneのAPIトークンを入力してください。</p>
      <label className="field">
        APIトークン
        <input type="password" value={token} onChange={(e) => setToken(e.target.value)} />
      </label>
      <button className="btn btn-primary" type="submit">
        ログイン
      </button>
    </form>
  )
}
