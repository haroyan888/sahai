// ヘルスチェック結果を表示するバッジ。

import type { HealthStatus } from '../api/types'

export interface HealthBadgeProps {
  health: HealthStatus
  /** trueの場合、文言を出さず色だけの小さいドットとして表示する(サービス名/コンテナ名の隣に添える用途)。 */
  dotOnly?: boolean
}

const LABELS: Record<HealthStatus, string> = {
  unknown: '不明',
  healthy: '正常',
  unhealthy: '異常',
}

export function HealthBadge({ health, dotOnly = false }: HealthBadgeProps) {
  return (
    <span
      className={dotOnly ? 'badge-dot' : 'badge'}
      data-testid="health-badge"
      data-health={health}
      title={dotOnly ? LABELS[health] : undefined}
    >
      {!dotOnly && LABELS[health]}
    </span>
  )
}
