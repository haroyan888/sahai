// NotServicePageの期待される振る舞いを先に定義する(TDDのRED)。
// 未登録サブドメインへのアクセス時に出る、ログイン不要のページ。

import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NotServicePage } from './NotServicePage'

const BASE_URL = 'https://sahai.example.com'

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  })
}

describe('NotServicePage', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn())
  })

  it('読み込み中はローディング表示をする', () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockReturnValue(new Promise(() => {}))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="mysql.example.com" />)

    expect(screen.getByText(/読み込み中/)).toBeInTheDocument()
  })

  it('hostnameをクエリパラメータとして/api/not-serviceを叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ found: false }))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="mysql.example.com" />)

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        `${BASE_URL}/api/not-service?host=mysql.example.com`,
      )
    })
  })

  it('found=trueの場合、サービス名・案内文・ポート一覧を表示する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        found: true,
        name: 'mysql',
        ports: [{ host_port: 20001, container_port: 3306, protocol: 'tcp' }],
      }),
    )

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="mysql.example.com" />)

    await waitFor(() => {
      expect(screen.getByText('mysql')).toBeInTheDocument()
      expect(screen.getByText(/下記のポートへ直接接続してください/)).toBeInTheDocument()
      expect(screen.getByText('20001')).toBeInTheDocument()
      expect(screen.getByText('3306')).toBeInTheDocument()
    })
  })

  it('found=falseの場合、サービスが見つからない旨を表示する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ found: false }))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="doesnotexist.example.com" />)

    await waitFor(() => {
      expect(screen.getByText(/見つかりません/)).toBeInTheDocument()
    })
  })

  it('取得に失敗した場合はエラーメッセージを表示する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockRejectedValueOnce(new Error('network error'))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="mysql.example.com" />)

    await waitFor(() => {
      expect(screen.getByText(/取得に失敗しました/)).toBeInTheDocument()
    })
  })
})
