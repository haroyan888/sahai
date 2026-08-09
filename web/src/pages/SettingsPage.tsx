// 設定画面。ルート: /settings。基本設定(domain/https_redirect/api_token、
// GET/PUT /api/settings)、DNS/証明書設定(DnsConfigSection、
// GET/PUT /api/settings/dns-provider)、レジストリ設定(RegistryConfigSection、
// GET/PUT /api/settings/registry)の3つの独立したセクションで構成する。
// 基本設定の保存後は即座にバックエンド側へ反映されるため、api_tokenが変更された場合は
// セッションを維持するためonTokenChangedで呼び出し元に通知する。

import { useEffect, useState } from 'react'
import { Save } from 'lucide-react'
import { DnsConfigSection } from '../components/DnsConfigSection'
import { FieldError } from '../components/FieldError'
import { RegistryConfigSection } from '../components/RegistryConfigSection'
import { parseApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { ApiErrorField, Settings } from '../api/types'

export interface SettingsPageProps {
  client: ApiClient
  onTokenChanged?: (token: string) => void
}

export function SettingsPage({ client, onTokenChanged }: SettingsPageProps) {
  const [settings, setSettings] = useState<Settings | null>(null)
  // ページを開いた時点のドメイン。保存後のドメインとここを比較し、変わっていれば
  // 「新しいURLへ移動」案内に切り替える(ドメイン変更はTraefikのルートを即座に
  // 書き換えるため、旧ドメインのままのこのページは自動的には再接続できない)。
  const [initialDomain, setInitialDomain] = useState<string | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [fieldErrors, setFieldErrors] = useState<ApiErrorField[]>([])
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const [newDomainUrl, setNewDomainUrl] = useState<string | null>(null)

  useEffect(() => {
    client
      .getSettings()
      .then((s) => {
        setSettings(s)
        setInitialDomain(s.domain)
      })
      .catch(() => setLoadError('取得に失敗しました'))
  }, [client])

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!settings) return
    setFieldErrors([])
    setError(null)
    setSaved(false)
    setNewDomainUrl(null)

    try {
      const result = await client.updateSettings(settings)
      setSettings(result)
      onTokenChanged?.(result.api_token)

      if (initialDomain !== null && result.domain !== initialDomain) {
        setNewDomainUrl(`https://sahai.${result.domain}/`)
      } else {
        setSaved(true)
      }
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

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev))
  }

  if (loadError) {
    return (
      <p className="alert" role="alert">
        {loadError}
      </p>
    )
  }

  if (!settings) {
    return <p>読み込み中...</p>
  }

  return (
    <>
      <form className="card" onSubmit={handleSubmit} data-testid="basic-settings-form">
        <h1>設定</h1>

        {error && (
          <p className="alert" role="alert">
            {error}
          </p>
        )}
        {newDomainUrl && (
          <p className="alert alert-success" role="status">
            保存しました。ドメインが変更されたため、このページのままでは操作を続けられません。新しいURL(
            <a href={newDomainUrl}>{newDomainUrl}</a>)へ移動してください。
          </p>
        )}
        {!newDomainUrl && saved && <p className="alert alert-success">保存しました</p>}

        <label className="field">
          ドメイン
          <input value={settings.domain} onChange={(e) => update('domain', e.target.value)} />
        </label>
        <FieldError field="domain" errors={fieldErrors} />

        <label className="field field-inline">
          <input
            type="checkbox"
            checked={settings.https_redirect}
            onChange={(e) => update('https_redirect', e.target.checked)}
          />
          HTTPSへリダイレクト
        </label>

        <label className="field">
          APIトークン
          <input value={settings.api_token} onChange={(e) => update('api_token', e.target.value)} />
        </label>
        <FieldError field="api_token" errors={fieldErrors} />

        <div className="actions">
          <button className="btn btn-primary" type="submit" title="基本設定を保存" aria-label="基本設定を保存">
            <Save size={16} />
          </button>
        </div>
      </form>

      <DnsConfigSection client={client} />
      <RegistryConfigSection client={client} />
    </>
  )
}
