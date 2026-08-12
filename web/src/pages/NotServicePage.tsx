// Not Serviceページ。ログイン不要で誰でも見られる。
// Traefikは非HTTPサービス・未登録サブドメイン宛てのアクセスをすべてWeb UIコンテナへ
// 転送する。このページはwindow.location.hostnameから
// アクセス元のサブドメインを判定し、/api/not-serviceへ問い合わせて案内を表示する。
// 未ログインでも見られる必要があるため、ApiClient(Bearer認証)は使わず素のfetchを使う。
//
// エンドユーザー向けの画面なので、どの状態でもh1に何が起きたか(エラー名)、
// pにその詳細、という同じ構成で表示する。

import { useEffect, useState } from 'react'
import type { NotServiceInfo } from '../api/types'

export interface NotServicePageProps {
  apiBaseUrl: string
  /** テスト容易化のため注入可能。省略時はwindow.location.hostnameを使う */
  hostname?: string
}

export function NotServicePage({ apiBaseUrl, hostname }: NotServicePageProps) {
  const [info, setInfo] = useState<NotServiceInfo | null>(null)
  const [error, setError] = useState(false)
  const host = hostname ?? window.location.hostname

  useEffect(() => {
    let cancelled = false
    setInfo(null)
    setError(false)
    fetch(`${apiBaseUrl}/api/not-service?host=${encodeURIComponent(host)}`)
      .then((res) => res.json() as Promise<NotServiceInfo>)
      .then((result) => {
        if (!cancelled) setInfo(result)
      })
      .catch(() => {
        if (!cancelled) setError(true)
      })
    return () => {
      cancelled = true
    }
  }, [apiBaseUrl, host])

  if (error) {
    return (
      <div className="card not-service-card">
        <h1>取得に失敗しました</h1>
        <p className="muted">
          {host} の状態を確認できませんでした。時間をおいて再度アクセスしてください。
        </p>
      </div>
    )
  }

  if (!info) {
    return (
      <div className="card not-service-card">
        <p className="muted">読み込み中...</p>
      </div>
    )
  }

  if (!info.found) {
    return (
      <div className="card not-service-card">
        <h1>サービスが見つかりません</h1>
        <p className="muted">
          {host} に対応するサービスは提供されていません。URLをご確認ください。
        </p>
      </div>
    )
  }

  return (
    <div className="card not-service-card">
      <h1>HTTP/HTTPSでは公開されていません</h1>
      <p className="muted">
        {info.name} はHTTP/HTTPSを提供していません。下記のポートへ直接接続してください。
      </p>
      {info.ports && info.ports.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>host_port</th>
              <th>container_port</th>
              <th>protocol</th>
            </tr>
          </thead>
          <tbody>
            {info.ports.map((p, i) => (
              <tr key={i}>
                <td>{p.host_port}</td>
                <td>{p.container_port}</td>
                <td>{p.protocol}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
