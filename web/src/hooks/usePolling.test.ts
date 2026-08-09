// usePollingの期待される振る舞いを先に定義する(TDDのRED)。
// ServiceListPage/ServiceDetailPageで繰り返されていた
// 「cancelledフラグ+setInterval+cleanup」パターンを共通化したもの。

import { renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { usePolling } from './usePolling'

describe('usePolling', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('マウント時に即座に1回fetcherを呼ぶ', () => {
    const fetcher = vi.fn()
    renderHook(() => usePolling(fetcher, 1000))

    expect(fetcher).toHaveBeenCalledTimes(1)
  })

  it('interval経過ごとにfetcherを呼び続ける', () => {
    const fetcher = vi.fn()
    renderHook(() => usePolling(fetcher, 1000))

    vi.advanceTimersByTime(3000)

    expect(fetcher).toHaveBeenCalledTimes(4) // 初回1 + 3回
  })

  it('アンマウント後はfetcherを呼ばなくなる', () => {
    const fetcher = vi.fn()
    const { unmount } = renderHook(() => usePolling(fetcher, 1000))
    unmount()

    vi.advanceTimersByTime(5000)

    expect(fetcher).toHaveBeenCalledTimes(1) // 初回のみ
  })

  it('渡すisCancelledは、アンマウント前はfalse、アンマウント後はtrueを返す', () => {
    let capturedIsCancelled: (() => boolean) | null = null
    const fetcher = vi.fn((isCancelled: () => boolean) => {
      capturedIsCancelled = isCancelled
    })
    const { unmount } = renderHook(() => usePolling(fetcher, 1000))

    expect(capturedIsCancelled!()).toBe(false)
    unmount()
    expect(capturedIsCancelled!()).toBe(true)
  })

  it('fetcherの参照が変わるとeffectが再実行され、onEffectStartが呼ばれる', () => {
    const onEffectStart = vi.fn()
    const { rerender } = renderHook(
      ({ fetcher }: { fetcher: () => void }) => usePolling(fetcher, 1000, onEffectStart),
      { initialProps: { fetcher: vi.fn() } },
    )
    expect(onEffectStart).toHaveBeenCalledTimes(1)

    rerender({ fetcher: vi.fn() })
    expect(onEffectStart).toHaveBeenCalledTimes(2)
  })
})
