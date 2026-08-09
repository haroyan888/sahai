// ServiceListPage/ServiceDetailPageで繰り返されていた「マウント時に即座に1回フェッチし、
// 以後intervalMsごとに再フェッチする。アンマウント/依存変化後に届いた古いレスポンスで
// stateを更新しないようcancelledフラグで守る」というポーリングパターンを共通化したもの。

import { useEffect } from 'react'

export type PollingFetcher = (isCancelled: () => boolean) => void

/**
 * @param fetcher 呼び出すたびに新しいフェッチを1回行う関数。`client`や`idOrName`など
 *   依存する値が変わったら呼び出し側で`useCallback`により参照を更新すること
 *   (このフックはfetcherの参照が変わるたびeffectを再実行し、ポーリングをリセットする)
 * @param intervalMs ポーリング間隔
 * @param onEffectStart effect開始時(マウント時・fetcherの参照が変わった時)に、
 *   最初のfetcher呼び出しより前に1回だけ呼ばれる。表示中のstateを「読み込み中」に
 *   戻すためのリセット処理を渡す想定
 */
export function usePolling(fetcher: PollingFetcher, intervalMs: number, onEffectStart?: () => void) {
  useEffect(() => {
    let cancelled = false
    const isCancelled = () => cancelled

    onEffectStart?.()
    fetcher(isCancelled)
    const interval = setInterval(() => fetcher(isCancelled), intervalMs)

    return () => {
      cancelled = true
      clearInterval(interval)
    }
  }, [fetcher, intervalMs, onEffectStart])
}
