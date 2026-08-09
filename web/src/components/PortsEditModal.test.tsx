// PortsEditModalの期待される振る舞いを先に定義する(TDDのRED)。
// モーダルの中に別のモーダルを開かない設計(画面遷移方針より)。

import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ApiError } from '../api/client'
import { PortsEditModal } from './PortsEditModal'
import type { ServiceContainer } from '../api/types'

function containers(): ServiceContainer[] {
  return [
    {
      id: 10,
      name: 'app',
      health_status: 'unknown',
      last_health_check_at: null,
      ports: [
        // HTTP公開のポートはホストに公開しないためhost_portを持たない
        { id: 100, container_port: 80, host_port: null, protocol: 'tcp', is_http: true },
        { id: 101, container_port: 3306, host_port: 20001, protocol: 'tcp', is_http: false },
      ],
      volumes: [{ id: 200, container_path: '/data' }],
    },
    {
      id: 11,
      name: 'db',
      health_status: 'unknown',
      last_health_check_at: null,
      ports: [],
      volumes: [{ id: 201, container_path: '/var/lib/mysql' }],
    },
  ]
}

describe('PortsEditModal', () => {
  it('role="dialog"として表示され、全コンテナ名を見出しとして表示する', () => {
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)
    const dialog = screen.getByRole('dialog')
    expect(within(dialog).getByText('app')).toBeInTheDocument()
    expect(within(dialog).getByText('db')).toBeInTheDocument()
  })

  it('各入力欄が何のデータかを示す説明・列見出しを表示する', () => {
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)

    const appSection = screen.getByTestId('container-section-app')
    // ポート欄: 列見出し(コンテナ内ポート/ホストポート/HTTP)と、意味を説明する補足文
    expect(within(appSection).getByText('コンテナ内ポート')).toBeInTheDocument()
    expect(within(appSection).getByText('ホストポート')).toBeInTheDocument()
    expect(within(appSection).getByText(/他のサービスと重複しない値/)).toBeInTheDocument()
    // ボリューム欄: 何を指定する欄かを説明する補足文
    expect(within(appSection).getByText(/永続化したいコンテナ内のパス/)).toBeInTheDocument()
  })

  it('既存のポート値を入力欄に表示する', () => {
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByDisplayValue('80')).toBeInTheDocument()
    expect(screen.getByDisplayValue('3306')).toBeInTheDocument()
    expect(screen.getByDisplayValue('20001')).toBeInTheDocument()
  })

  it('HTTP公開のポートにはホストポート欄を出さない', () => {
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)
    // 非HTTPのポート1件分だけ入力欄がある
    expect(screen.getAllByLabelText(/appのホストポート/)).toHaveLength(1)
    expect(screen.getByText('不要')).toBeInTheDocument()
  })

  it('既存のボリューム値を入力欄に表示する', () => {
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByDisplayValue('/data')).toBeInTheDocument()
    expect(screen.getByDisplayValue('/var/lib/mysql')).toBeInTheDocument()
  })

  it('「ポート追加」でその場に空の行が増える(別モーダルを開かない)', async () => {
    const user = userEvent.setup()
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)

    const appSection = screen.getByTestId('container-section-app')
    expect(within(appSection).getAllByTestId('port-row')).toHaveLength(2)

    await user.click(within(appSection).getByRole('button', { name: 'ポートを追加' }))

    expect(within(appSection).getAllByTestId('port-row')).toHaveLength(3)
    // 追加時点でダイアログは1つのままである(サブモーダルを開かない)
    expect(screen.getAllByRole('dialog')).toHaveLength(1)
  })

  it('ポート行の削除ボタンで該当行が消える', async () => {
    const user = userEvent.setup()
    render(<PortsEditModal containers={containers()} onSave={vi.fn()} onClose={vi.fn()} />)

    const appSection = screen.getByTestId('container-section-app')
    await user.click(within(appSection).getAllByRole('button', { name: 'ポートを削除' })[0])

    expect(within(appSection).queryAllByTestId('port-row')).toHaveLength(1)
  })

  it('保存ボタンでonSaveにContainerInput形式(name/ports/volumes)を渡す', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    render(<PortsEditModal containers={containers()} onSave={onSave} onClose={vi.fn()} />)

    await user.click(screen.getByRole('button', { name: '保存' }))

    expect(onSave).toHaveBeenCalledWith([
      expect.objectContaining({
        name: 'app',
        ports: [
          expect.objectContaining({ container_port: 80, host_port: null, protocol: 'tcp', is_http: true }),
          expect.objectContaining({ container_port: 3306, host_port: 20001, protocol: 'tcp', is_http: false }),
        ],
        volumes: [{ container_path: '/data' }],
      }),
      expect.objectContaining({
        name: 'db',
        ports: [],
        volumes: [{ container_path: '/var/lib/mysql' }],
      }),
    ])
  })

  it('保存が失敗したらモーダルを閉じず、該当ポートの欄にエラーを表示する', async () => {
    const user = userEvent.setup()
    // サーバはfieldにcontainers[i].ports[j].host_portを付けて返す
    const onSave = vi.fn().mockRejectedValue(
      new ApiError(400, 'VALIDATION_ERROR', '入力内容を確認してください', [
        { field: 'containers[0].ports[0].host_port', message: "ポート20001はサービス'other'が使用中です" },
      ]),
    )
    const onClose = vi.fn()
    render(<PortsEditModal containers={containers()} onSave={onSave} onClose={onClose} />)

    await user.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByText(/サービス'other'が使用中です/)).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('入力内容を確認してください')
    // 値を直せるようモーダルは開いたままにする
    expect(onClose).not.toHaveBeenCalled()
  })

  it('保存が失敗しても、フィールド情報が無ければ汎用のメッセージを出す', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn().mockRejectedValue(new Error('network down'))
    render(<PortsEditModal containers={containers()} onSave={onSave} onClose={vi.fn()} />)

    await user.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('保存できませんでした')
  })

  it('閉じるボタンはonCloseのみ呼びonSaveは呼ばない', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    const onClose = vi.fn()
    render(<PortsEditModal containers={containers()} onSave={onSave} onClose={onClose} />)

    await user.click(screen.getByRole('button', { name: '閉じる' }))

    expect(onClose).toHaveBeenCalled()
    expect(onSave).not.toHaveBeenCalled()
  })
})
