// サービス一覧画面。ルート: /services。

import { useCallback, useState } from 'react'
import { Link } from 'react-router-dom'
import { Play, Plus, RotateCw, Square } from 'lucide-react'
import type { ApiClient } from '../api/client'
import type { Service } from '../api/types'
import { HealthBadge } from '../components/HealthBadge'
import { usePolling } from '../hooks/usePolling'

// サーバー側は10秒間隔でヘルス判定するため、それより短い間隔で取りに行く
// (WebSocket等は使わず、単純なポーリングで済ませる)
const POLL_INTERVAL_MS = 5000

export interface ServiceListPageProps {
  client: ApiClient
}

type ServiceAction = 'start' | 'stop' | 'restart'

export function ServiceListPage({ client }: ServiceListPageProps) {
  const [services, setServices] = useState<Service[] | null>(null)
  const [error, setError] = useState(false)
  // サービス名をキーに、実行中の操作を保持する。同時に複数サービスを
  // 操作していても互いに影響しないようRecordで持つ
  const [actionInProgress, setActionInProgress] = useState<Record<string, ServiceAction>>({})

  const fetchServices = useCallback(
    (isCancelled: () => boolean) => {
      client
        .listServices()
        .then((result) => {
          if (!isCancelled()) {
            setServices(result)
            setError(false)
          }
        })
        .catch(() => {
          if (!isCancelled()) setError(true)
        })
    },
    [client],
  )
  const resetServices = useCallback(() => {
    setServices(null)
    setError(false)
  }, [])
  usePolling(fetchServices, POLL_INTERVAL_MS, resetServices)

  async function handleAction(name: string, action: ServiceAction) {
    setActionInProgress((prev) => ({ ...prev, [name]: action }))
    try {
      if (action === 'start') await client.startService(name)
      else if (action === 'stop') await client.stopService(name)
      else await client.restartService(name)

      const updated = await client.listServices()
      setServices(updated)
    } finally {
      setActionInProgress((prev) => {
        const next = { ...prev }
        delete next[name]
        return next
      })
    }
  }

  return (
    <div className="page-fill entity-list">
      <div className="entity-list__header">
        <h1 style={{ marginBottom: 0 }}>Services</h1>
        <Link className="btn btn-icon btn-primary" to="/services/new" title="新規登録" aria-label="新規登録">
          <Plus size={16} aria-hidden="true" />
        </Link>
      </div>

      {services === null && !error && (
        <div className="entity-list-empty">
          <p className="muted">読み込み中...</p>
        </div>
      )}
      {error && (
        <div className="entity-list-empty">
          <p className="alert">取得に失敗しました</p>
        </div>
      )}
      {services && services.length === 0 && (
        <div className="entity-list-empty">
          <p className="muted">登録されているサービスがありません</p>
        </div>
      )}

      {services && services.length > 0 && (
        <ul className="entity-list__body">
          {services.map((service) => {
            const inProgress = actionInProgress[service.name]
            return (
              <li className="entity-list__item" key={service.id}>
                <span className="row" style={{ gap: 'var(--space-2)' }}>
                  <Link to={`/services/${service.name}`}>{service.name}</Link>
                  {service.status !== 'stopped' && <HealthBadge health={service.health_status} dotOnly />}
                </span>
                <span className="entity-list__spacer" />
                <div className="row" style={{ marginLeft: 'var(--space-2)' }}>
                  {service.status === 'stopped' && (
                    <button
                      className="btn btn-icon btn-success"
                      type="button"
                      title="起動"
                      aria-label={inProgress === 'start' ? '起動中...' : '起動'}
                      disabled={inProgress !== undefined}
                      onClick={() => void handleAction(service.name, 'start')}
                    >
                      <Play size={16} aria-hidden="true" />
                    </button>
                  )}
                  {service.status !== 'stopped' && (
                    <>
                      <button
                        className="btn btn-icon"
                        type="button"
                        title="停止"
                        aria-label={inProgress === 'stop' ? '停止中...' : '停止'}
                        disabled={inProgress !== undefined}
                        onClick={() => void handleAction(service.name, 'stop')}
                      >
                        <Square size={16} aria-hidden="true" />
                      </button>
                      <button
                        className="btn btn-icon"
                        type="button"
                        title="再起動"
                        aria-label={inProgress === 'restart' ? '再起動中...' : '再起動'}
                        disabled={inProgress !== undefined}
                        onClick={() => void handleAction(service.name, 'restart')}
                      >
                        <RotateCw size={16} aria-hidden="true" />
                      </button>
                    </>
                  )}
                </div>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
