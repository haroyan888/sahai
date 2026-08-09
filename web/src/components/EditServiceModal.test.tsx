// 環境変数の`.env`ファイル読み込みを中心とした振る舞いの確認。

import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { EditServiceModal } from './EditServiceModal'
import type { ServiceDetail } from '../api/types'

function detail(envVars: Record<string, string> = {}): ServiceDetail {
  return {
    id: 1,
    name: 'myapp',
    subdomain: 'myapp.example.com',
    source_type: 'image',
    image: 'registry.sahai.example.com/myapp:latest',
    compose_content: null,
    env_vars: envVars,
    status: 'stopped',
    health_status: 'unknown',
    last_health_check_at: null,
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
    containers: [],
  }
}

function envFile(content: string): File {
  return new File([content], '.env', { type: 'text/plain' })
}

async function upload(content: string) {
  const input = screen.getByLabelText('.envファイルから読み込む')
  await userEvent.upload(input, envFile(content))
}

describe('EditServiceModal の .env 読み込み', () => {
  it('読み込んだ環境変数が入力欄に反映される', async () => {
    render(<EditServiceModal detail={detail()} onSave={vi.fn()} onClose={vi.fn()} />)

    await upload('FOO=bar\nBAZ=qux\n')

    const keys = screen.getAllByLabelText('環境変数キー') as HTMLInputElement[]
    const values = screen.getAllByLabelText('環境変数の値') as HTMLInputElement[]
    expect(keys.map((i) => i.value)).toEqual(['FOO', 'BAZ'])
    expect(values.map((i) => i.value)).toEqual(['bar', 'qux'])
  })

  it('既存の環境変数を消さず、同じキーだけ上書きする', async () => {
    render(<EditServiceModal detail={detail({ KEEP: '1', OVERWRITE: 'old' })} onSave={vi.fn()} onClose={vi.fn()} />)

    await upload('OVERWRITE=new\nADDED=2\n')

    const keys = screen.getAllByLabelText('環境変数キー') as HTMLInputElement[]
    const values = screen.getAllByLabelText('環境変数の値') as HTMLInputElement[]
    expect(keys.map((i) => i.value)).toEqual(['KEEP', 'OVERWRITE', 'ADDED'])
    expect(values.map((i) => i.value)).toEqual(['1', 'new', '2'])
  })

  it('読み込んだ件数と、保存が必要な旨を知らせる', async () => {
    render(<EditServiceModal detail={detail()} onSave={vi.fn()} onClose={vi.fn()} />)

    await upload('FOO=bar\nBAZ=qux\n')

    expect(screen.getByRole('status')).toHaveTextContent('2件')
    expect(screen.getByRole('status')).toHaveTextContent('保存するまで反映されません')
  })

  it('読み取れる行が無い場合はその旨を知らせ、入力欄を変更しない', async () => {
    render(<EditServiceModal detail={detail()} onSave={vi.fn()} onClose={vi.fn()} />)

    await upload('# コメントだけ\n\n')

    expect(screen.getByRole('status')).toHaveTextContent('読み取れる環境変数がありませんでした')
    expect(screen.queryAllByLabelText('環境変数キー')).toHaveLength(0)
  })

  it('読み込み後に保存すると、その内容がonSaveへ渡る', async () => {
    const onSave = vi.fn()
    render(<EditServiceModal detail={detail()} onSave={onSave} onClose={vi.fn()} />)

    await upload('FOO=bar\n')
    await userEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ env_vars: { FOO: 'bar' } }))
  })
})
