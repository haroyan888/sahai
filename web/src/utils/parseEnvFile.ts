// `.env`ファイルの内容をキー・値のペアへ変換する。
// サーバー側の`.sahai.env`パーサ(env_file.rs)と同じく、コメント行・空行を無視し
// 最初の`=`で分割する。加えて、一般的な`.env`で使われる`export `接頭辞と
// 値を囲む引用符にも対応する。

export interface EnvVarRow {
  key: string
  value: string
}

/** キーとして妥当か(英字またはアンダースコア始まり、英数字とアンダースコアのみ)。 */
function isValidKey(key: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key)
}

function unquote(value: string): string {
  if (value.length >= 2) {
    const head = value[0]
    if ((head === '"' || head === "'") && value.endsWith(head)) {
      return value.slice(1, -1)
    }
  }
  return value
}

/**
 * `.env`テキストをパースする。解釈できない行は黙って無視する
 * (利用者が用意したファイルに未知の記法が混ざっていても、読める分だけ取り込む)。
 */
export function parseEnvFile(content: string): EnvVarRow[] {
  const rows: EnvVarRow[] = []
  for (const line of content.split(/\r?\n/)) {
    const trimmed = line.trim()
    if (trimmed === '' || trimmed.startsWith('#')) continue

    const withoutExport = trimmed.startsWith('export ') ? trimmed.slice('export '.length).trim() : trimmed
    const eq = withoutExport.indexOf('=')
    if (eq <= 0) continue

    const key = withoutExport.slice(0, eq).trim()
    if (!isValidKey(key)) continue

    rows.push({ key, value: unquote(withoutExport.slice(eq + 1).trim()) })
  }
  return rows
}

/**
 * 既存の入力行に、読み込んだ行を反映する。同じキーは上書きし、無いキーは末尾へ追加する。
 * 既存の手入力を消さずに済むため、ファイル読み込みを何度でもやり直せる。
 */
export function mergeEnvVarRows(existing: EnvVarRow[], loaded: EnvVarRow[]): EnvVarRow[] {
  const merged = [...existing]
  for (const row of loaded) {
    const index = merged.findIndex((e) => e.key === row.key)
    if (index >= 0) {
      merged[index] = { ...merged[index], value: row.value }
    } else {
      merged.push(row)
    }
  }
  return merged
}
