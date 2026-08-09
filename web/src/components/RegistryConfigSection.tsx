// 設定画面内の「レジストリ設定」セクション。GET/PUT /api/settings/registry を扱う。
// sahai service create(プロジェクトをサーバーへアップロードし、サーバー側でdocker
// build/pushするコマンド)が使うレジストリURL・資格情報を保存する。保存すると
// サーバーが同期的にdocker loginを試みるが、失敗してもDB保存自体は成功する
// (DNS/証明書設定のTraefik再作成失敗時とは異なる設計)。docker loginは
// 接続断を起こさない軽い処理のため、DNS/証明書設定のような再接続待ちの仕組みは持たず、
// 「基本設定」カードに近いシンプルなsavedフラグ方式にする。

import { useEffect, useState } from 'react'
import { Save } from 'lucide-react'
import { FieldError } from './FieldError'
import { parseApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { ApiErrorField, RegistryConfig } from '../api/types'

export interface RegistryConfigSectionProps {
  client: ApiClient
}

export function RegistryConfigSection({ client }: RegistryConfigSectionProps) {
  const [registryUrl, setRegistryUrl] = useState('')
  const [registryUsername, setRegistryUsername] = useState('')
  const [registryPassword, setRegistryPassword] = useState('')
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [fieldErrors, setFieldErrors] = useState<ApiErrorField[]>([])
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const [loginWarning, setLoginWarning] = useState<string | null>(null)

  useEffect(() => {
    client
      .getRegistryConfig()
      .then((config) => {
        setRegistryUrl(config.registry_url)
        setRegistryUsername(config.registry_username ?? '')
        setRegistryPassword(config.registry_password ?? '')
        setLoading(false)
      })
      .catch(() => {
        setLoadError('取得に失敗しました')
        setLoading(false)
      })
  }, [client])

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setFieldErrors([])
    setError(null)
    setSaved(false)
    setLoginWarning(null)

    const payload: RegistryConfig = {
      registry_url: registryUrl,
      registry_username: registryUsername === '' ? null : registryUsername,
      registry_password: registryPassword === '' ? null : registryPassword,
    }

    try {
      const result = await client.updateRegistryConfig(payload)
      setRegistryUrl(result.registry_url)
      setRegistryUsername(result.registry_username ?? '')
      setRegistryPassword(result.registry_password ?? '')
      setSaved(true)
      setLoginWarning(result.login_warning ?? null)
    } catch (err) {
      const parsed = parseApiError(err)
      if (parsed) {
        setFieldErrors(parsed.fields)
        setError(parsed.message)
      } else {
        setError('保存に失敗しました')
      }
    }
  }

  if (loadError) {
    return (
      <p className="alert" role="alert">
        {loadError}
      </p>
    )
  }

  if (loading) {
    return <p>読み込み中...</p>
  }

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h2>レジストリ設定(拡張設定)</h2>
      <p className="muted">
        sahai service create(プロジェクトをアップロードしてサーバー側でビルド・pushするコマンド)が使う、
        Dockerレジストリの接続先と資格情報です。<strong>通常はsetup.sh/setup.ps1の初回セットアップ時に自動設定されるため、変更は不要です。</strong>
        パスワードをローテーションしたい場合や、同梱のregistry:2コンテナではなく外部のレジストリへ切り替えたい場合にのみここで編集してください。
        保存すると、サーバーが即座にdocker loginを試みます。ログインに失敗しても設定自体は保存されます。ユーザー名・パスワードは両方入力するか、
        両方空のままにしてください。
      </p>

      {error && (
        <p className="alert" role="alert">
          {error}
        </p>
      )}
      {saved && <p className="alert alert-success">保存しました</p>}
      {loginWarning && (
        <p className="alert" role="alert">
          {loginWarning}。設定は保存されています。
        </p>
      )}

      <label className="field">
        レジストリURL
        <input value={registryUrl} onChange={(e) => setRegistryUrl(e.target.value)} />
      </label>
      <p className="muted">空欄ならドメインから自動生成します(registry.sahai.&lt;ドメイン&gt;)</p>
      <FieldError field="registry_url" errors={fieldErrors} />

      <label className="field">
        ユーザー名
        <input value={registryUsername} onChange={(e) => setRegistryUsername(e.target.value)} />
      </label>
      <FieldError field="registry_username" errors={fieldErrors} />

      <label className="field">
        パスワード
        <input
          type="password"
          value={registryPassword}
          onChange={(e) => setRegistryPassword(e.target.value)}
        />
      </label>
      <FieldError field="registry_password" errors={fieldErrors} />

      <div className="actions">
        <button className="btn btn-primary" type="submit" title="レジストリ設定を保存" aria-label="レジストリ設定を保存">
          <Save size={16} />
        </button>
      </div>
    </form>
  )
}
