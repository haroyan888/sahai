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

  // どの状態でもh1に何が起きたか(エラー名)、pにその詳細、という構成で表示する。
  it('found=trueの場合、h1に見出し・pにサービス名を含む詳細・ポート一覧を表示する', async () => {
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
      expect(
        screen.getByRole('heading', { name: 'HTTP/HTTPSでは公開されていません' }),
      ).toBeInTheDocument()
      expect(
        screen.getByText(/mysql はHTTP\/HTTPSを提供していません。下記のポートへ直接接続してください。/),
      ).toBeInTheDocument()
      expect(screen.getByText('20001')).toBeInTheDocument()
      expect(screen.getByText('3306')).toBeInTheDocument()
    })
  })

  it('found=falseの場合、h1に見出し・pにホスト名を含む詳細を表示する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ found: false }))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="doesnotexist.example.com" />)

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'サービスが見つかりません' })).toBeInTheDocument()
      expect(
        screen.getByText(/doesnotexist.example.com に対応するサービスは提供されていません。/),
      ).toBeInTheDocument()
    })
  })

  it('取得に失敗した場合、h1に見出し・pに詳細を表示する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockRejectedValueOnce(new Error('network error'))

    render(<NotServicePage apiBaseUrl={BASE_URL} hostname="mysql.example.com" />)

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: '取得に失敗しました' })).toBeInTheDocument()
      expect(screen.getByText(/mysql.example.com の状態を確認できませんでした。/)).toBeInTheDocument()
    })
  })
})
