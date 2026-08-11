// サービス詳細画面。
// ルート: /services/:name。
//
// 編集は画面遷移を増やさない方針(ユーザー確認済み):
// - name/image/compose_content/env_varsは1階層のみのモーダル(EditServiceModal)で編集
// - containers(ports/volumes)は1階層のみのモーダル(PortsEditModal)で編集し、
//   モーダルの中に別のモーダルは開かない

import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronDown, MoreVertical, Pencil, Play, RotateCw, Settings2, Square, Trash2 } from 'lucide-react'
import type { ApiClient } from '../api/client'
import type {
  ContainerInput,
  HealthResponse,
  RegistryStatusResponse,
  ServiceDetail,
  StatsResponse,
  UpdateServiceRequest,
} from '../api/types'
import { HealthBadge } from '../components/HealthBadge'
import { PortsEditModal } from '../components/PortsEditModal'
import { EditServiceModal } from '../components/EditServiceModal'
import { DeleteConfirmModal } from '../components/DeleteConfirmModal'
import { ContainerLogsPanel } from '../components/ContainerLogsPanel'
import { formatBytes } from '../utils/formatBytes'
import { usePolling } from '../hooks/usePolling'

// サーバー側は10秒間隔でヘルス判定するため、それより短い間隔で取りに行く
// (WebSocket等は使わず、単純なポーリングで済ませる)
const POLL_INTERVAL_MS = 5000

export interface ServiceDetailPageProps {
  client: ApiClient
  idOrName: string
  /** 削除確定後に呼ばれる(呼び出し側で一覧画面への遷移に使う想定)。 */
  onDeleted?: () => void
}

export function ServiceDetailPage({ client, idOrName, onDeleted }: ServiceDetailPageProps) {
  const [detail, setDetail] = useState<ServiceDetail | null>(null)
  const [error, setError] = useState(false)

  const [deleteModalOpen, setDeleteModalOpen] = useState(false)

  const [editModalOpen, setEditModalOpen] = useState(false)
  const [portsModalOpen, setPortsModalOpen] = useState(false)

  // 編集/削除をまとめた「その他の操作」メニュー。メニュー外をクリックしたら閉じる
  const [menuOpen, setMenuOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!menuOpen) return
    function handleClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [menuOpen])

  // start/stop/restartは完了までAPIレスポンス待ちになるため、ボタンを押してから
  // 結果が反映されるまでの間、処理中であることが分かるよう表示・操作を無効化する
  const [actionInProgress, setActionInProgress] = useState<'start' | 'stop' | 'restart' | null>(null)

  const [health, setHealth] = useState<HealthResponse | null>(null)
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [registryStatus, setRegistryStatus] = useState<RegistryStatusResponse | null>(null)

  const fetchDetail = useCallback(
    (isCancelled: () => boolean) => {
      client
        .getService(idOrName)
        .then((result) => {
          if (!isCancelled()) {
            setDetail(result)
            setError(false)
          }
        })
        .catch(() => {
          if (!isCancelled()) setError(true)
        })
    },
    [client, idOrName],
  )
  const resetDetail = useCallback(() => {
    setDetail(null)
    setError(false)
  }, [])
  usePolling(fetchDetail, POLL_INTERVAL_MS, resetDetail)

  // ヘルス/統計情報は補助的な表示のため、失敗しても詳細画面自体は表示し続ける
  const fetchHealthAndStats = useCallback(
    (isCancelled: () => boolean) => {
      client
        .getHealth(idOrName)
        .then((result) => {
          if (!isCancelled()) setHealth(result)
        })
        .catch(() => {})
      client
        .getStats(idOrName)
        .then((result) => {
          if (!isCancelled()) setStats(result)
        })
        .catch(() => {})
      client
        .getRegistryStatus(idOrName)
        .then((result) => {
          if (!isCancelled()) setRegistryStatus(result)
        })
        .catch(() => {})
    },
    [client, idOrName],
  )
  const resetHealthAndStats = useCallback(() => {
    setHealth(null)
    setStats(null)
    setRegistryStatus(null)
  }, [])
  usePolling(fetchHealthAndStats, POLL_INTERVAL_MS, resetHealthAndStats)

  if (error) {
    return <p className="alert">取得に失敗しました</p>
  }

  if (!detail) {
    return <p className="muted">読み込み中...</p>
  }

  async function handleStart() {
    setActionInProgress('start')
    try {
      const updated = await client.startService(idOrName)
      setDetail(updated)
    } finally {
      setActionInProgress(null)
    }
  }

  async function handleStop() {
    setActionInProgress('stop')
    try {
      const updated = await client.stopService(idOrName)
      setDetail(updated)
    } finally {
      setActionInProgress(null)
    }
  }

  async function handleRestart() {
    setActionInProgress('restart')
    try {
      const updated = await client.restartService(idOrName)
      setDetail(updated)
    } finally {
      setActionInProgress(null)
    }
  }

  async function handleConfirmDelete() {
    await client.deleteService(idOrName, false)
    setDeleteModalOpen(false)
    onDeleted?.()
  }

  async function handleSaveEdit(payload: UpdateServiceRequest) {
    const updated = await client.updateService(idOrName, payload)
    setDetail(updated)
    setEditModalOpen(false)
  }

  // 失敗はそのまま呼び出し元(PortsEditModal)へ投げ返す。
  // モーダルを開いたままエラーを表示させ、利用者が値を直せるようにするため。
  async function handleSavePorts(containers: ContainerInput[]) {
    const updated = await client.updateService(idOrName, { containers })
    setDetail(updated)
    setPortsModalOpen(false)
  }

  return (
    <div>
      <div className="card">
        {detail.status === 'error' && detail.last_error && (
          <div className="alert" role="alert">
            <strong>起動に失敗しました</strong>
            <pre className="alert-detail">{detail.last_error}</pre>
          </div>
        )}
        {detail.route_warning && (
          <p className="alert" role="alert">
            {detail.route_warning}
          </p>
        )}
        <div className="row row--between">
          <div>
            <div className="row" style={{ gap: 'var(--space-2)' }}>
              <h1 style={{ marginBottom: 0 }}>{detail.name}</h1>
              {detail.status !== 'stopped' && <HealthBadge health={detail.health_status} dotOnly />}
            </div>
            <p className="muted">{detail.subdomain}</p>
          </div>
          <div className="row">
            {detail.status === 'stopped' && (
              <button
                className="btn btn-icon btn-success"
                type="button"
                title="起動"
                aria-label={actionInProgress === 'start' ? '起動中...' : '起動'}
                disabled={actionInProgress !== null}
                onClick={() => void handleStart()}
              >
                <Play size={16} aria-hidden="true" />
              </button>
            )}
            {detail.status !== 'stopped' && (
              <>
                <button
                  className="btn btn-icon"
                  type="button"
                  title="停止"
                  aria-label={actionInProgress === 'stop' ? '停止中...' : '停止'}
                  disabled={actionInProgress !== null}
                  onClick={() => void handleStop()}
                >
                  <Square size={16} aria-hidden="true" />
                </button>
                <button
                  className="btn btn-icon"
                  type="button"
                  title="再起動"
                  aria-label={actionInProgress === 'restart' ? '再起動中...' : '再起動'}
                  disabled={actionInProgress !== null}
                  onClick={() => void handleRestart()}
                >
                  <RotateCw size={16} aria-hidden="true" />
                </button>
              </>
            )}
            <div className="dropdown" ref={menuRef}>
              <button
                className="btn btn-icon"
                type="button"
                title="その他の操作"
                aria-label="その他の操作"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                onClick={() => setMenuOpen((v) => !v)}
              >
                <MoreVertical size={16} aria-hidden="true" />
              </button>
              {menuOpen && (
                <div className="dropdown-menu">
                  <button
                    className="dropdown-menu__item"
                    type="button"
                    onClick={() => {
                      setMenuOpen(false)
                      setEditModalOpen(true)
                    }}
                  >
                    <Pencil size={16} /> 編集
                  </button>
                  <button
                    className="dropdown-menu__item dropdown-menu__item--danger"
                    type="button"
                    onClick={() => {
                      setMenuOpen(false)
                      setDeleteModalOpen(true)
                    }}
                  >
                    <Trash2 size={16} /> 削除
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {editModalOpen && (
        <EditServiceModal
          detail={detail}
          onSave={(payload) => void handleSaveEdit(payload)}
          onClose={() => setEditModalOpen(false)}
        />
      )}

      {deleteModalOpen && (
        <DeleteConfirmModal
          serviceName={detail.name}
          onConfirm={() => void handleConfirmDelete()}
          onClose={() => setDeleteModalOpen(false)}
        />
      )}

      <section className="card">
        <h2>コンテナ</h2>
        <div className="actions" style={{ marginTop: 0, marginBottom: 'var(--space-3)' }}>
          <button
            className="btn btn-sm"
            type="button"
            title="ポート/ボリュームを編集"
            aria-label="ポート/ボリュームを編集"
            onClick={() => setPortsModalOpen(true)}
          >
            <Settings2 size={16} />
          </button>
        </div>
        <div className="stack">
          {detail.containers.map((container) => {
            const containerHealth = health?.containers.find((c) => c.id === container.id)
            const containerStats = stats?.containers.find((c) => c.id === container.id)
            const containerRegistry = registryStatus?.containers.find((c) => c.id === container.id)
            return (
              <details className="subcard" key={container.id}>
                <summary className="row row--between">
                  <span className="row" style={{ gap: 'var(--space-2)' }}>
                    <strong>コンテナ: {container.name}</strong>
                    {containerHealth && (
                      <span data-testid={`container-health-${container.id}`}>
                        <HealthBadge health={containerHealth.health_status} dotOnly />
                      </span>
                    )}
                  </span>
                  <ChevronDown className="subcard-chevron" size={16} aria-hidden="true" />
                </summary>
                <h4>ポートマッピング</h4>
                <ul className="kv-list">
                  {container.ports.map((port) => (
                    <li key={port.id}>
                      {port.container_port} → {port.host_port}
                    </li>
                  ))}
                  {container.volumes.map((volume) => (
                    <li key={volume.id}>{volume.container_path}</li>
                  ))}
                </ul>
                {containerStats && (
                  <>
                    <h4 style={{ marginTop: 'var(--space-3)' }}>資源利用状況</h4>
                    <p className="muted" data-testid={`container-stats-${container.id}`}>
                      CPU {containerStats.cpu_percent}% / メモリ {formatBytes(containerStats.memory_usage_bytes)}/
                      {formatBytes(containerStats.memory_limit_bytes)}
                    </p>
                  </>
                )}
                {containerRegistry && (
                  <>
                    <h4 style={{ marginTop: 'var(--space-3)' }}>レジストリ</h4>
                    <p className="muted" data-testid={`container-registry-${container.id}`}>
                      <code>{containerRegistry.image_tag}</code>{' '}
                      {containerRegistry.image_present ? (
                        <span style={{ color: 'var(--success)' }}>登録済み</span>
                      ) : (
                        <span style={{ color: 'var(--danger)' }}>未登録</span>
                      )}
                    </p>
                  </>
                )}
              </details>
            )
          })}
        </div>
      </section>

      {detail.containers.length > 0 && (
        <ContainerLogsPanel client={client} idOrName={idOrName} containers={detail.containers} />
      )}

      {portsModalOpen && (
        <PortsEditModal
          containers={detail.containers}
          onSave={handleSavePorts}
          onClose={() => setPortsModalOpen(false)}
        />
      )}
    </div>
  )
}
