// 設定画面内の「DNS/証明書設定」セクション。GET/PUT /api/settings/dns-provider を扱う。
// 保存するとバックエンド側でTraefikコンテナが再作成され、数秒間フロントからの接続が
// 途切れる(ドメイン自体は変わらないため、同じURLのまま自動的に再接続できる)。
// そのためupdateDnsConfig呼び出し後は、たとえ通信エラーになっても「保存自体は
// 成功している可能性がある」とみなして再接続待ち状態に遷移し、getDnsConfigの
// ポーリングで接続が戻るのを確認する。バリデーションエラー(Traefikに触れる前に
// 同期的に返る400)のみ、再接続待ちに入らずその場でフィールドエラーを表示する。

import { useEffect, useState } from 'react'
import { Plus, Save, Trash2 } from 'lucide-react'
import { FieldError } from './FieldError'
import { parseApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { ApiErrorField, DnsConfig, DnsCredential } from '../api/types'

const POLL_INTERVAL_MS = 2000
// バックエンド側のTraefik再作成リトライ(最大48秒程度かかりうる。
// crates/sahai-server/src/traefik/container.rs参照)を安全に上回る待ち時間にする
const MAX_POLL_ATTEMPTS = 30

export interface DnsConfigSectionProps {
  client: ApiClient
}

type Phase = 'loading' | 'idle' | 'saving' | 'reconnecting' | 'reconnect-failed' | 'saved'

const BUSY_PHASES: Phase[] = ['saving', 'reconnecting']

export function DnsConfigSection({ client }: DnsConfigSectionProps) {
  const [dnsProvider, setDnsProvider] = useState('')
  const [acmeEmail, setAcmeEmail] = useState('')
  const [credentials, setCredentials] = useState<DnsCredential[]>([])
  const [loadError, setLoadError] = useState<string | null>(null)
  const [fieldErrors, setFieldErrors] = useState<ApiErrorField[]>([])
  const [error, setError] = useState<string | null>(null)
  const [phase, setPhase] = useState<Phase>('loading')
  const [pollAttempt, setPollAttempt] = useState(0)

  useEffect(() => {
    client
      .getDnsConfig()
      .then((config) => {
        setDnsProvider(config.dns_provider)
        setAcmeEmail(config.acme_email)
        setCredentials(config.credentials)
        setPhase('idle')
      })
      .catch(() => {
        setLoadError('取得に失敗しました')
      })
  }, [client])

  useEffect(() => {
    if (phase !== 'reconnecting') return

    setPollAttempt(0)
    let attempt = 0
    let cancelled = false
    let timer: ReturnType<typeof setTimeout>

    function poll() {
      client
        .getDnsConfig()
        .then(() => {
          if (!cancelled) setPhase('saved')
        })
        .catch(() => {
          if (cancelled) return
          attempt += 1
          setPollAttempt(attempt)
          if (attempt >= MAX_POLL_ATTEMPTS) {
            setPhase('reconnect-failed')
          } else {
            timer = setTimeout(poll, POLL_INTERVAL_MS)
          }
        })
    }

    timer = setTimeout(poll, POLL_INTERVAL_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [phase, client])

  // 経過秒数はpollAttempt(何回ポーリングに失敗したか)から逆算する。
  // ポーリング成功直後の1回だけタイマーを待たずに済むため厳密な経過時間ではないが、
  // 「固まっていないこと」を利用者に伝える目安としては十分
  const elapsedSeconds = pollAttempt * (POLL_INTERVAL_MS / 1000)

  function addCredentialRow() {
    setCredentials((prev) => [...prev, { key: '', value: '' }])
  }

  function removeCredentialRow(index: number) {
    setCredentials((prev) => prev.filter((_, i) => i !== index))
  }

  function updateCredentialRow(index: number, patch: Partial<DnsCredential>) {
    setCredentials((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setFieldErrors([])
    setError(null)
    setPhase('saving')

    const payload: DnsConfig = { dns_provider: dnsProvider, acme_email: acmeEmail, credentials }

    try {
      await client.updateDnsConfig(payload)
    } catch (err) {
      const parsed = parseApiError(err)
      if (parsed && parsed.fields.length > 0) {
        setFieldErrors(parsed.fields)
        setError(parsed.message)
        setPhase('idle')
        return
      }
      // バリデーション以外のエラーはTraefik再作成中の接続断による可能性があるため、
      // 保存失敗として扱わずそのまま再接続待ちに進む。
    }
    setPhase('reconnecting')
  }

  if (loadError) {
    return (
      <p className="alert" role="alert">
        {loadError}
      </p>
    )
  }

  if (phase === 'loading') {
    return <p>読み込み中...</p>
  }

  const busy = BUSY_PHASES.includes(phase)

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h2>DNS/証明書設定</h2>
      <p className="muted">
        証明書を自動発行するためのDNSプロバイダ設定です。保存するとこの画面への接続が一時的に切れ、
        自動で再接続します(数秒〜最大1分)。発行済みの証明書やサービスへのルーティングは失われません。
      </p>

      {error && (
        <p className="alert" role="alert">
          {error}
        </p>
      )}
      {phase === 'reconnecting' && (
        <p className="alert" role="status">
          再接続を待っています(経過: {elapsedSeconds}秒)。最大1分ほどかかります。
        </p>
      )}
      {phase === 'reconnect-failed' && (
        <p className="alert" role="alert">
          再接続を確認できませんでした。設定は保存されている可能性があります。ページを再読み込みしてください。
        </p>
      )}
      {phase === 'saved' && <p className="alert alert-success">保存し、再接続を確認しました</p>}

      <label className="field">
        DNSプロバイダ
        <input value={dnsProvider} onChange={(e) => setDnsProvider(e.target.value)} disabled={busy} />
      </label>
      <FieldError field="dns_provider" errors={fieldErrors} />

      <label className="field">
        ACME通知先メールアドレス
        <input value={acmeEmail} onChange={(e) => setAcmeEmail(e.target.value)} disabled={busy} />
      </label>
      <FieldError field="acme_email" errors={fieldErrors} />

      <h3>認証情報</h3>
      <div className="stack">
        {credentials.map((row, index) => (
          <div className="port-row" data-testid="dns-credential-row" key={index}>
            <input
              type="text"
              aria-label="認証情報キー"
              value={row.key}
              onChange={(e) => updateCredentialRow(index, { key: e.target.value })}
              style={{ width: 'auto', flex: 1 }}
              disabled={busy}
            />
            <input
              type="text"
              aria-label="認証情報の値"
              value={row.value}
              onChange={(e) => updateCredentialRow(index, { value: e.target.value })}
              style={{ width: 'auto', flex: 1 }}
              disabled={busy}
            />
            <button
              className="btn btn-danger btn-sm"
              type="button"
              title="認証情報を削除"
              aria-label="認証情報を削除"
              onClick={() => removeCredentialRow(index)}
              disabled={busy}
            >
              <Trash2 size={16} />
            </button>
          </div>
        ))}
      </div>
      <FieldError field="credentials" errors={fieldErrors} matchPrefix />
      <div className="actions">
        <button
          className="btn btn-sm"
          type="button"
          title="認証情報を追加"
          aria-label="認証情報を追加"
          onClick={addCredentialRow}
          disabled={busy}
        >
          <Plus size={16} />
        </button>
      </div>

      <div className="actions">
        <button className="btn btn-primary" type="submit" title="DNS設定を保存" aria-label="DNS設定を保存" disabled={busy}>
          <Save size={16} />
        </button>
      </div>
    </form>
  )
}
