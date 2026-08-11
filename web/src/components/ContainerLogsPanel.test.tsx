import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ContainerLogsPanel } from './ContainerLogsPanel'
import { ApiError } from '../api/client'
import type { ApiClient, LogStreamHandlers, LogStreamOptions } from '../api/client'
import type { ServiceContainer } from '../api/types'

function container(overrides: Partial<ServiceContainer> = {}): ServiceContainer {
  return {
    id: 10,
    name: 'web',
    health_status: 'unknown',
    last_health_check_at: null,
    ports: [],
    volumes: [],
    ...overrides,
  }
}

/**
 * streamLogsは接続が切れるまで解決しないため、テスト側からは
 * 「渡されたハンドラを掴んで、任意のタイミングで行を流す」形で操作する。
 */
function streamingClient() {
  const captured: { options: LogStreamOptions; handlers: LogStreamHandlers }[] = []
  const streamLogs = vi.fn(
    (_idOrName: string, options: LogStreamOptions, handlers: LogStreamHandlers) => {
      captured.push({ options, handlers })
      return new Promise<void>(() => {})
    },
  )
  return { client: { streamLogs } as unknown as ApiClient, captured, streamLogs }
}

describe('ContainerLogsPanel', () => {
  it('受け取った行を表示する', async () => {
    const { client, captured } = streamingClient()
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    await waitFor(() => expect(captured).toHaveLength(1))
    captured[0].handlers.onLine({
      stream: 'stdout',
      timestamp: '2026-08-11T07:05:25.472263Z',
      message: 'Listening on :3000',
    })

    expect(await screen.findByText('Listening on :3000')).toBeInTheDocument()
  })

  /** 異常を追うのが主目的なので、標準エラーだと分かる必要がある。 */
  it('標準エラーの行を標準出力と区別できる', async () => {
    const { client, captured } = streamingClient()
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    await waitFor(() => expect(captured).toHaveLength(1))
    captured[0].handlers.onLine({ stream: 'stderr', timestamp: null, message: 'connection refused' })

    const line = await screen.findByText('connection refused')
    expect(line.closest('.log-line')).toHaveAttribute('data-stream', 'stderr')
  })

  it('最初のコンテナを既定で表示する', async () => {
    const { client, captured } = streamingClient()
    render(
      <ContainerLogsPanel
        client={client}
        idOrName="myapp"
        containers={[container({ id: 10 }), container({ id: 11, name: 'db' })]}
      />,
    )

    await waitFor(() => expect(captured).toHaveLength(1))
    expect(captured[0].options.container).toBe(10)
  })

  it('コンテナを切り替えると選んだコンテナで接続し直す', async () => {
    const user = userEvent.setup()
    const { client, captured } = streamingClient()
    render(
      <ContainerLogsPanel
        client={client}
        idOrName="myapp"
        containers={[container({ id: 10 }), container({ id: 11, name: 'db' })]}
      />,
    )

    await waitFor(() => expect(captured).toHaveLength(1))
    await user.selectOptions(screen.getByLabelText('ログを表示するコンテナ'), '11')

    await waitFor(() => expect(captured).toHaveLength(2))
    expect(captured[1].options.container).toBe(11)
  })

  /** コンテナが1つなら選ばせる意味がない。 */
  it('コンテナが1つなら選択欄を出さない', async () => {
    const { client } = streamingClient()
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    expect(screen.queryByLabelText('ログを表示するコンテナ')).not.toBeInTheDocument()
  })

  it('追従を止めると読み出しを中断する', async () => {
    const user = userEvent.setup()
    const { client, captured } = streamingClient()
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    await waitFor(() => expect(captured).toHaveLength(1))
    expect(captured[0].options.signal.aborted).toBe(false)

    await user.click(screen.getByRole('button', { name: '追従を止める' }))

    expect(captured[0].options.signal.aborted).toBe(true)
    expect(await screen.findByRole('button', { name: '追従を再開する' })).toBeInTheDocument()
  })

  it('画面から外れたら読み出しを止める', async () => {
    const { client, captured } = streamingClient()
    const { unmount } = render(
      <ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />,
    )

    await waitFor(() => expect(captured).toHaveLength(1))
    unmount()

    expect(captured[0].options.signal.aborted).toBe(true)
  })

  it('サーバーが送るerrorイベントを表示する', async () => {
    const { client, captured } = streamingClient()
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    await waitFor(() => expect(captured).toHaveLength(1))
    captured[0].handlers.onServerError('コンテナが見つかりません')

    expect(await screen.findByText('コンテナが見つかりません')).toBeInTheDocument()
  })

  it('接続自体に失敗したらエラーを表示する', async () => {
    const client = {
      streamLogs: vi.fn().mockRejectedValue(new ApiError(404, 'NOT_FOUND', 'サービスが見つかりません')),
    } as unknown as ApiClient
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    expect(await screen.findByText('サービスが見つかりません')).toBeInTheDocument()
  })

  /** 停止・画面遷移で必ずabortするため、これをエラー表示すると毎回出てしまう。 */
  it('中断によるエラーは表示しない', async () => {
    const user = userEvent.setup()
    const captured: LogStreamOptions[] = []
    const client = {
      streamLogs: vi.fn((_idOrName: string, options: LogStreamOptions) => {
        captured.push(options)
        return new Promise<void>((_resolve, reject) => {
          options.signal.addEventListener('abort', () => reject(new Error('aborted')))
        })
      }),
    } as unknown as ApiClient
    render(<ContainerLogsPanel client={client} idOrName="myapp" containers={[container()]} />)

    await waitFor(() => expect(captured).toHaveLength(1))
    await user.click(screen.getByRole('button', { name: '追従を止める' }))

    await waitFor(() => expect(captured[0].signal.aborted).toBe(true))
    expect(screen.queryByText('ログを取得できませんでした')).not.toBeInTheDocument()
  })
})
