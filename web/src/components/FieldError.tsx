// バリデーションエラー(ApiErrorField[])の中から特定フィールド分だけを絞り込んで
// 表示する共通コンポーネント。各フォーム画面で同じフィルタ+表示パターンが
// 繰り返されていたため抽出した。

import type { ApiErrorField } from '../api/types'

export interface FieldErrorProps {
  /** 絞り込むフィールド名。matchPrefixがtrueの場合はこの文字列で始まるものすべて */
  field: string
  errors: ApiErrorField[]
  /** trueの場合、`credentials[0].key`のようなインデックス付きパスを前方一致で拾う */
  matchPrefix?: boolean
}

export function FieldError({ field, errors, matchPrefix = false }: FieldErrorProps) {
  const matches = errors.filter((f) => (matchPrefix ? f.field.startsWith(field) : f.field === field))

  return (
    <>
      {matches.map((f) => (
        <p className="field-error" key={f.field}>
          {f.message}
        </p>
      ))}
    </>
  )
}
