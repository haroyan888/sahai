// FieldErrorの期待される振る舞いを先に定義する(TDDのRED)。

import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { FieldError } from './FieldError'
import type { ApiErrorField } from '../api/types'

describe('FieldError', () => {
  it('fieldが完全一致するエラーメッセージだけを表示する', () => {
    const errors: ApiErrorField[] = [
      { field: 'domain', message: 'ドメインを入力してください' },
      { field: 'api_token', message: 'APIトークンを入力してください' },
    ]
    render(<FieldError field="domain" errors={errors} />)

    expect(screen.getByText('ドメインを入力してください')).toBeInTheDocument()
    expect(screen.queryByText('APIトークンを入力してください')).not.toBeInTheDocument()
  })

  it('一致するエラーが無ければ何も表示しない', () => {
    const errors: ApiErrorField[] = [{ field: 'domain', message: 'ドメインを入力してください' }]
    const { container } = render(<FieldError field="api_token" errors={errors} />)

    expect(container).toBeEmptyDOMElement()
  })

  it('matchPrefix指定時はfieldの前方一致でエラーメッセージを表示する(インデックス付きパス対応)', () => {
    const errors: ApiErrorField[] = [
      { field: 'credentials[0].key', message: 'キーを入力してください' },
      { field: 'dns_provider', message: 'DNSプロバイダを入力してください' },
    ]
    render(<FieldError field="credentials" errors={errors} matchPrefix />)

    expect(screen.getByText('キーを入力してください')).toBeInTheDocument()
    expect(screen.queryByText('DNSプロバイダを入力してください')).not.toBeInTheDocument()
  })
})
