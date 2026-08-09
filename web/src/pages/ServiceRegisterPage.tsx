// サービス新規登録画面。ルート: /services/new。

import { useState } from 'react'
import { FieldError } from '../components/FieldError'
import { parseApiError } from '../api/client'
import type { ApiClient } from '../api/client'
import type { ApiErrorField, SourceType, ServiceDetail } from '../api/types'

export interface ServiceRegisterPageProps {
  client: ApiClient
  onCreated?: (detail: ServiceDetail) => void
}

export function ServiceRegisterPage({ client, onCreated }: ServiceRegisterPageProps) {
  const [name, setName] = useState('')
  const [sourceType, setSourceType] = useState<SourceType>('image')
  const [image, setImage] = useState('')
  const [composeContent, setComposeContent] = useState('')
  const [fieldErrors, setFieldErrors] = useState<ApiErrorField[]>([])
  const [error, setError] = useState<string | null>(null)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setFieldErrors([])
    setError(null)

    try {
      const detail = await client.createService(
        sourceType === 'image'
          ? {
              name,
              source_type: 'image',
              image,
              containers: [{ name }],
            }
          : {
              // バックエンドがcompose_contentをパースして全コンテナ(ports/volumes空)を
              // 自動登録するため、containersは空でよい。ports/volumesの設定は登録後に
              // 詳細画面のPortsEditModalで行う
              name,
              source_type: 'compose',
              compose_content: composeContent,
              containers: [],
            },
      )
      onCreated?.(detail)
    } catch (err) {
      const parsed = parseApiError(err)
      if (parsed) {
        setFieldErrors(parsed.fields)
        setError(parsed.message)
      } else {
        setError('登録に失敗しました')
      }
    }
  }

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h1>サービス新規登録</h1>

      {error && (
        <p className="alert" role="alert">
          {error}
        </p>
      )}

      <label className="field">
        サービス名
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <FieldError field="name" errors={fieldErrors} />

      <fieldset>
        <legend>ソース種別</legend>
        <div className="row">
          <label className="field field-inline">
            <input
              type="radio"
              name="source_type"
              checked={sourceType === 'image'}
              onChange={() => setSourceType('image')}
            />
            Image
          </label>
          <label className="field field-inline">
            <input
              type="radio"
              name="source_type"
              checked={sourceType === 'compose'}
              onChange={() => setSourceType('compose')}
            />
            Compose
          </label>
        </div>
      </fieldset>

      {sourceType === 'image' && (
        <>
          <label className="field">
            イメージ
            <input value={image} onChange={(e) => setImage(e.target.value)} />
          </label>
          <FieldError field="image" errors={fieldErrors} />
        </>
      )}

      {sourceType === 'compose' && (
        <>
          <label className="field">
            compose_content
            <textarea value={composeContent} onChange={(e) => setComposeContent(e.target.value)} />
          </label>
          <FieldError field="compose_content" errors={fieldErrors} />
          <p className="muted">
            各サービスがそのままコンテナになります。ポート/ボリュームは登録後に設定します。
          </p>
          <p className="muted">
            compose内の <code>ports:</code> と <code>env_file:</code> は無視されます。
            公開ポートと環境変数は、この画面で登録した後の設定が使われます。
            <code>environment:</code> に直接書いた値はそのまま残ります。
          </p>
          <p className="muted">
            <code>sahai service create</code> でビルドする場合、composeファイルはプロジェクトルート直下に置いてください
            (<code>build.context</code> もルート配下しか指せません)。
          </p>
        </>
      )}

      <div className="actions">
        <button className="btn btn-primary" type="submit">
          登録
        </button>
      </div>
    </form>
  )
}
