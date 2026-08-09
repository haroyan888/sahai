import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { HealthBadge } from './HealthBadge'
import type { HealthStatus } from '../api/types'

describe('HealthBadge', () => {
  it.each([
    ['unknown', '不明'],
    ['healthy', '正常'],
    ['unhealthy', '異常'],
  ] as [HealthStatus, string][])('health=%s のとき "%s" と表示する', (health, expected) => {
    render(<HealthBadge health={health} />)
    expect(screen.getByTestId('health-badge')).toHaveTextContent(expected)
  })

  it('data-health属性にhealthの値をそのまま持たせる(スタイリング用フック)', () => {
    render(<HealthBadge health="unhealthy" />)
    expect(screen.getByTestId('health-badge')).toHaveAttribute('data-health', 'unhealthy')
  })

  it('dotOnly指定時は文言を出さず、title属性でラベルを持たせる', () => {
    render(<HealthBadge health="unhealthy" dotOnly />)
    const badge = screen.getByTestId('health-badge')
    expect(badge).toHaveTextContent('')
    expect(badge).toHaveAttribute('title', '異常')
    expect(badge).toHaveAttribute('data-health', 'unhealthy')
  })
})
