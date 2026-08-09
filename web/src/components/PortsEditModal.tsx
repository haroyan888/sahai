// containers(ports/volumes)をまとめて編集するモーダル。
// 画面遷移方針(ユーザー確認済み)により、このモーダルの中に別のモーダルは開かない
// (ポート/ボリュームの追加削除はモーダル内のインラインリスト操作で完結させる)。
// ApiClientは直接扱わず、保存内容をonSaveで親(ServiceDetailPage)に渡すだけの
// controlled component。

import { useState } from 'react'
import { Plus, Save, Trash2, X } from 'lucide-react'
import { parseApiError } from '../api/client'
import type { ApiErrorField, ContainerInput, PortInput, Protocol, ServiceContainer, VolumeInput } from '../api/types'
import { FieldError } from './FieldError'

export interface PortsEditModalProps {
  containers: ServiceContainer[]
  /** 保存の成否を待つため、親は結果をPromiseで返す。失敗時はこのモーダルが表示を受け持つ */
  onSave: (containers: ContainerInput[]) => Promise<void>
  onClose: () => void
}

interface EditableContainer {
  name: string
  ports: PortInput[]
  volumes: VolumeInput[]
}

function toEditable(containers: ServiceContainer[]): EditableContainer[] {
  return containers.map((c) => ({
    name: c.name,
    ports: c.ports.map((p) => ({
      container_port: p.container_port,
      host_port: p.host_port,
      protocol: p.protocol,
      is_http: p.is_http,
    })),
    volumes: c.volumes.map((v) => ({ container_path: v.container_path })),
  }))
}

export function PortsEditModal({ containers, onSave, onClose }: PortsEditModalProps) {
  const [editable, setEditable] = useState<EditableContainer[]>(() => toEditable(containers))
  const [fieldErrors, setFieldErrors] = useState<ApiErrorField[]>([])
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  function updatePort(containerIndex: number, portIndex: number, patch: Partial<PortInput>) {
    setEditable((prev) =>
      prev.map((c, ci) =>
        ci !== containerIndex
          ? c
          : { ...c, ports: c.ports.map((p, pi) => (pi === portIndex ? { ...p, ...patch } : p)) },
      ),
    )
  }

  function addPort(containerIndex: number) {
    setEditable((prev) =>
      prev.map((c, ci) =>
        ci !== containerIndex
          ? c
          : {
              ...c,
              // host_portに範囲の制限は無いが、既定値は衝突しにくい値にしておく
              ports: [...c.ports, { container_port: 0, host_port: 20000, protocol: 'tcp' as Protocol, is_http: false }],
            },
      ),
    )
  }

  function removePort(containerIndex: number, portIndex: number) {
    setEditable((prev) =>
      prev.map((c, ci) => (ci !== containerIndex ? c : { ...c, ports: c.ports.filter((_, pi) => pi !== portIndex) })),
    )
  }

  function updateVolume(containerIndex: number, volumeIndex: number, containerPath: string) {
    setEditable((prev) =>
      prev.map((c, ci) =>
        ci !== containerIndex
          ? c
          : {
              ...c,
              volumes: c.volumes.map((v, vi) => (vi === volumeIndex ? { container_path: containerPath } : v)),
            },
      ),
    )
  }

  async function handleSave() {
    const result: ContainerInput[] = editable.map((c) => ({
      name: c.name,
      ports: c.ports,
      volumes: c.volumes,
    }))
    setFieldErrors([])
    setError(null)
    setSaving(true)
    try {
      await onSave(result)
    } catch (err) {
      // 保存できなかったときはモーダルを閉じない。利用者が値を直してやり直せるようにする
      const parsed = parseApiError(err)
      setFieldErrors(parsed?.fields ?? [])
      setError(parsed?.message ?? '保存できませんでした')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="modal-overlay">
      <div className="modal" role="dialog" aria-label="ポート/ボリュームを編集">
        <button className="modal-close" type="button" title="閉じる" aria-label="閉じる" onClick={onClose}>
          <X size={18} />
        </button>
        {editable.map((container, containerIndex) => (
          <section
            className="modal-section"
            key={container.name}
            data-testid={`container-section-${container.name}`}
          >
            <h3>{container.name}</h3>

            <div>
              <h4>ポート</h4>
              <p className="muted">
                ホストポートは他のサービスと重複しない値を選びます。
                「HTTP」を付けたポートが <code>&lt;サービス名&gt;.&lt;ドメイン&gt;</code> で公開されます(1コンテナに1つまで)。
              </p>
              {container.ports.length > 0 && (
                <div className="port-row port-row-header muted" aria-hidden="true">
                  <span className="port-col-label">コンテナ内ポート</span>
                  <span className="port-col-label">ホストポート</span>
                  <span>HTTP公開</span>
                </div>
              )}
              {container.ports.map((port, portIndex) => (
                <div key={portIndex}>
                <div className="port-row" data-testid="port-row">
                  <input
                    type="number"
                    aria-label={`${container.name}のコンテナ内ポート`}
                    title="コンテナが実際に待ち受けるポート番号"
                    value={port.container_port}
                    onChange={(e) =>
                      updatePort(containerIndex, portIndex, { container_port: Number(e.target.value) })
                    }
                  />
                  <input
                    type="number"
                    aria-label={`${container.name}のホストポート`}
                    title="外部からの接続先となるポート番号"
                    value={port.host_port}
                    onChange={(e) => updatePort(containerIndex, portIndex, { host_port: Number(e.target.value) })}
                  />
                  <label className="field field-inline" title="このポートをサブドメイン経由で公開する(1コンテナにつき1つまで)">
                    <input
                      type="checkbox"
                      aria-label={`${container.name}のHTTP公開`}
                      checked={port.is_http ?? false}
                      onChange={(e) => updatePort(containerIndex, portIndex, { is_http: e.target.checked })}
                    />
                  </label>
                  <button
                    className="btn btn-danger btn-sm"
                    type="button"
                    title="ポートを削除"
                    aria-label="ポートを削除"
                    onClick={() => removePort(containerIndex, portIndex)}
                  >
                    <Trash2 size={16} />
                  </button>
                </div>
                <FieldError
                  field={`containers[${containerIndex}].ports[${portIndex}].host_port`}
                  errors={fieldErrors}
                />
                </div>
              ))}
              <button
                className="btn btn-sm"
                type="button"
                title="ポートを追加"
                aria-label="ポートを追加"
                onClick={() => addPort(containerIndex)}
              >
                <Plus size={16} />
              </button>
            </div>

            <div>
              <h4>ボリューム</h4>
              <p className="muted">
                永続化したいコンテナ内のパスを絶対パスで指定します。ホスト側の保存先は自動で割り当てられます。
              </p>
              {container.volumes.map((volume, volumeIndex) => (
                <div className="port-row" key={volumeIndex}>
                  <input
                    type="text"
                    aria-label={`${container.name}のボリュームパス`}
                    title="コンテナ内の永続化したいディレクトリパス"
                    placeholder="/data"
                    value={volume.container_path}
                    onChange={(e) => updateVolume(containerIndex, volumeIndex, e.target.value)}
                    style={{ width: 'auto', flex: 1 }}
                  />
                </div>
              ))}
            </div>
          </section>
        ))}

        {error && (
          <p className="alert" role="alert">
            {error}
          </p>
        )}
        <div className="actions">
          <button
            className="btn btn-primary"
            type="button"
            title="保存"
            aria-label="保存"
            disabled={saving}
            onClick={() => void handleSave()}
          >
            <Save size={16} />
          </button>
        </div>
      </div>
    </div>
  )
}
