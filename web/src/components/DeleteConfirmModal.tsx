// サービス削除の確認モーダル。誤操作防止のため即削除せず、
// 必ずこのモーダルでの確認を経由させる。

import { X } from 'lucide-react'

export interface DeleteConfirmModalProps {
  serviceName: string
  onConfirm: () => void
  onClose: () => void
}

export function DeleteConfirmModal({ serviceName, onConfirm, onClose }: DeleteConfirmModalProps) {
  return (
    <div className="modal-overlay">
      <div className="modal" role="dialog" aria-label="サービスを削除">
        <button className="modal-close" type="button" title="閉じる" aria-label="閉じる" onClick={onClose}>
          <X size={18} />
        </button>
        <p>「{serviceName}」を削除しますか?</p>
        {/* 取り返しのつかない操作のため、ここだけはアイコンにせず文字で明示する */}
        <div className="actions">
          <button className="btn btn-danger" type="button" onClick={onConfirm}>
            削除を確定
          </button>
        </div>
      </div>
    </div>
  )
}
