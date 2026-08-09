// サービス名/イメージ(またはcompose_content)/環境変数を編集するモーダル。
// 画面遷移方針(ユーザー確認済み)により、このモーダルの中に別のモーダルは開かない。
// ApiClientは直接扱わず、保存内容をonSaveで親(ServiceDetailPage)に渡すだけの
// controlled component。

import { useState } from 'react'
import { Plus, Save, Trash2, Upload, X } from 'lucide-react'
import type { ServiceDetail, UpdateServiceRequest } from '../api/types'
import { mergeEnvVarRows, parseEnvFile } from '../utils/parseEnvFile'

export interface EditServiceModalProps {
  detail: ServiceDetail
  onSave: (payload: UpdateServiceRequest) => void
  onClose: () => void
}

export function EditServiceModal({ detail, onSave, onClose }: EditServiceModalProps) {
  const [draftName, setDraftName] = useState(detail.name)
  const [draftImage, setDraftImage] = useState(detail.image ?? '')
  const [draftComposeContent, setDraftComposeContent] = useState(detail.compose_content ?? '')
  const [draftEnvVars, setDraftEnvVars] = useState<{ key: string; value: string }[]>(
    Object.entries(detail.env_vars).map(([key, value]) => ({ key, value })),
  )
  const [envFileMessage, setEnvFileMessage] = useState<string | null>(null)

  function addEnvVarRow() {
    setDraftEnvVars((prev) => [...prev, { key: '', value: '' }])
  }

  function removeEnvVarRow(index: number) {
    setDraftEnvVars((prev) => prev.filter((_, i) => i !== index))
  }

  function updateEnvVarRow(index: number, patch: Partial<{ key: string; value: string }>) {
    setDraftEnvVars((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }

  // .envファイルを読み込んで入力欄へ反映する。
  // 読み込んだ時点では下書きに載せるだけで、保存は既存の「保存」ボタンに委ねる
  async function handleEnvFileSelected(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0]
    // 同じファイルを選び直しても再度onChangeが発火するよう、値をクリアしておく
    e.target.value = ''
    if (!file) return

    setEnvFileMessage(null)
    try {
      const loaded = parseEnvFile(await file.text())
      if (loaded.length === 0) {
        setEnvFileMessage(`${file.name} から読み取れる環境変数がありませんでした。`)
        return
      }
      setDraftEnvVars((prev) => mergeEnvVarRows(prev, loaded))
      setEnvFileMessage(`${file.name} から${loaded.length}件読み込みました(保存するまで反映されません)。`)
    } catch {
      setEnvFileMessage(`${file.name} の読み込みに失敗しました。`)
    }
  }

  function handleSave() {
    const payload: UpdateServiceRequest = {}
    if (draftName !== detail.name) payload.name = draftName
    if (detail.source_type === 'image' && draftImage !== (detail.image ?? '')) {
      payload.image = draftImage
    }
    if (detail.source_type === 'compose' && draftComposeContent !== (detail.compose_content ?? '')) {
      payload.compose_content = draftComposeContent
    }

    const envVars = Object.fromEntries(
      draftEnvVars.filter((row) => row.key.trim() !== '').map((row) => [row.key, row.value]),
    )
    if (JSON.stringify(envVars) !== JSON.stringify(detail.env_vars)) {
      payload.env_vars = envVars
    }

    onSave(payload)
  }

  return (
    <div className="modal-overlay">
      <form
        className="modal"
        role="dialog"
        aria-label="サービスを編集"
        onSubmit={(e) => {
          e.preventDefault()
          handleSave()
        }}
      >
        <button className="modal-close" type="button" title="閉じる" aria-label="閉じる" onClick={onClose}>
          <X size={18} />
        </button>
        <label className="field">
          サービス名
          <input value={draftName} onChange={(e) => setDraftName(e.target.value)} />
        </label>

        {detail.source_type === 'image' && (
          <label className="field">
            イメージ
            <input value={draftImage} onChange={(e) => setDraftImage(e.target.value)} />
          </label>
        )}

        {detail.source_type === 'compose' && (
          <label className="field">
            compose_content
            <textarea value={draftComposeContent} onChange={(e) => setDraftComposeContent(e.target.value)} />
          </label>
        )}

        <h3>環境変数</h3>
        <div className="stack">
          {draftEnvVars.map((row, index) => (
            <div className="port-row" data-testid="env-var-row" key={index}>
              <input
                type="text"
                aria-label="環境変数キー"
                value={row.key}
                onChange={(e) => updateEnvVarRow(index, { key: e.target.value })}
                style={{ width: 'auto', flex: 1 }}
              />
              <input
                type="text"
                aria-label="環境変数の値"
                value={row.value}
                onChange={(e) => updateEnvVarRow(index, { value: e.target.value })}
                style={{ width: 'auto', flex: 1 }}
              />
              <button
                className="btn btn-danger btn-sm"
                type="button"
                title="環境変数を削除"
                aria-label="環境変数を削除"
                onClick={() => removeEnvVarRow(index)}
              >
                <Trash2 size={16} />
              </button>
            </div>
          ))}
        </div>
        <div className="actions">
          <button
            className="btn btn-sm"
            type="button"
            title="環境変数を追加"
            aria-label="環境変数を追加"
            onClick={addEnvVarRow}
          >
            <Plus size={16} />
          </button>
          <label className="btn btn-sm" title=".envファイルから読み込む">
            <Upload size={16} />
            <input
              type="file"
              accept=".env,text/plain"
              aria-label=".envファイルから読み込む"
              onChange={handleEnvFileSelected}
              style={{ display: 'none' }}
            />
          </label>
        </div>
        {envFileMessage && <p role="status">{envFileMessage}</p>}

        <div className="actions">
          <button className="btn btn-primary" type="submit" title="保存" aria-label="保存">
            <Save size={16} />
          </button>
        </div>
      </form>
    </div>
  )
}
