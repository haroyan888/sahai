import { useCallback, useEffect, useRef, useState } from 'react'
import { Eraser, Pause, Play } from 'lucide-react'
import type { ApiClient } from '../api/client'
import { parseApiError } from '../api/client'
import type { LogLine, ServiceContainer } from '../api/types'

/**
 * 接続時に受け取る行数。多すぎると開いた瞬間に固まるため、画面を埋める程度に留める。
 */
const TAIL = 200

/**
 * 画面に保持する行数の上限。ログを出し続けるコンテナを開きっぱなしにすると
 * 際限なく溜まるため、古い行から捨てる。
 */
const MAX_LINES = 2000

/** 一番下から何px以内なら「最新を追っている」とみなすか。 */
const STICK_THRESHOLD_PX = 32

interface ContainerLogsPanelProps {
  client: ApiClient
  idOrName: string
  containers: ServiceContainer[]
}

export function ContainerLogsPanel({ client, idOrName, containers }: ContainerLogsPanelProps) {
  const [containerId, setContainerId] = useState<number | undefined>(containers[0]?.id)
  const [lines, setLines] = useState<LogLine[]>([])
  const [following, setFollowing] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const viewportRef = useRef<HTMLDivElement>(null)
  // 過去を読み返している最中に新しい行で下へ飛ばされないよう、
  // 利用者が一番下にいるときだけ追従する
  const stickToBottom = useRef(true)

  useEffect(() => {
    if (!following || containerId === undefined) return

    const controller = new AbortController()
    let cancelled = false
    setLines([])
    setError(null)

    client
      .streamLogs(
        idOrName,
        { container: containerId, tail: TAIL, signal: controller.signal },
        {
          onLine(line) {
            if (cancelled) return
            setLines((prev) => {
              const next = [...prev, line]
              return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next
            })
          },
          onServerError(message) {
            if (!cancelled) setError(message)
          },
        },
      )
      .catch((err: unknown) => {
        // 停止操作・画面遷移でのabortは異常ではないため何も出さない
        if (cancelled || controller.signal.aborted) return
        setError(parseApiError(err)?.message ?? 'ログを取得できませんでした')
      })

    return () => {
      cancelled = true
      controller.abort()
    }
  }, [client, idOrName, containerId, following])

  useEffect(() => {
    const viewport = viewportRef.current
    if (viewport && stickToBottom.current) {
      viewport.scrollTop = viewport.scrollHeight
    }
  }, [lines])

  const handleScroll = useCallback(() => {
    const viewport = viewportRef.current
    if (!viewport) return
    const distanceFromBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
    stickToBottom.current = distanceFromBottom <= STICK_THRESHOLD_PX
  }, [])

  return (
    <section className="card">
      <h2>ログ</h2>
      <div className="row row--between" style={{ marginBottom: 'var(--space-3)' }}>
        {containers.length > 1 ? (
          <label className="field field-inline" style={{ marginBottom: 0, gap: 'var(--space-2)' }}>
            コンテナ
            <select
              value={containerId ?? ''}
              onChange={(e) => setContainerId(Number(e.target.value))}
              aria-label="ログを表示するコンテナ"
            >
              {containers.map((container) => (
                <option key={container.id} value={container.id}>
                  {container.name}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <span className="muted">コンテナ: {containers[0]?.name ?? '(なし)'}</span>
        )}
        <span className="actions" style={{ margin: 0 }}>
          <button
            className="btn btn-sm"
            type="button"
            title={following ? '追従を止める' : '追従を再開する'}
            aria-label={following ? '追従を止める' : '追従を再開する'}
            onClick={() => setFollowing((prev) => !prev)}
          >
            {following ? <Pause size={16} /> : <Play size={16} />}
          </button>
          <button
            className="btn btn-sm"
            type="button"
            title="表示を消す"
            aria-label="表示を消す"
            onClick={() => setLines([])}
          >
            <Eraser size={16} />
          </button>
        </span>
      </div>

      {error && <div className="alert">{error}</div>}

      <div className="log-viewport" ref={viewportRef} onScroll={handleScroll} data-testid="log-viewport">
        {lines.length === 0 ? (
          <p className="muted" style={{ margin: 0 }}>
            {following ? 'ログを待っています...' : '追従を停止しています。'}
          </p>
        ) : (
          lines.map((line, index) => (
            // 同じ内容の行が連続することは普通にあるため、キーは表示順とする
            // (行は末尾への追加と先頭からの切り捨てしか起きない)
            <div className="log-line" data-stream={line.stream} key={index}>
              {line.timestamp && <span className="log-line__time">{formatTime(line.timestamp)}</span>}
              <span className="log-line__message">{line.message}</span>
            </div>
          ))
        )}
      </div>
    </section>
  )
}

/**
 * 秒までの時刻に切り詰める。日付まで出すと1行が長くなり、ログ本体が読みにくい。
 * 解釈できない値はそのまま出す(Dockerの出力を勝手に捨てない)。
 */
function formatTime(timestamp: string): string {
  const parsed = new Date(timestamp)
  if (Number.isNaN(parsed.getTime())) return timestamp
  return parsed.toLocaleTimeString('ja-JP', { hour12: false })
}
