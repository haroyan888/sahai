// Web UIのバンドルに入る依存パッケージの著作権表示を web/THIRD-PARTY-LICENSES.md に集める。
// npmパッケージを更新したら実行し直すこと。
//
// 対象は package-lock.json で dev:true が付いていないものだけ。ビルドツールや
// テスト基盤は配布物に入らないため、記載すると逆に何を配っているかが分かりにくくなる。
//
// 使い方: node scripts/gen-web-licenses.mjs

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'web')
const lock = JSON.parse(fs.readFileSync(path.join(webDir, 'package-lock.json'), 'utf8'))

const packages = Object.entries(lock.packages)
  .filter(([p, info]) => p.startsWith('node_modules/') && !info.dev)
  .map(([p, info]) => ({
    name: p.slice('node_modules/'.length),
    version: info.version,
    license: info.license,
    dir: path.join(webDir, p),
  }))
  .sort((a, b) => a.name.localeCompare(b.name))

if (packages.length === 0) {
  console.error('依存が見つかりません。web/でnpm installを実行してから再試行してください。')
  process.exit(1)
}

const missing = []
const sections = packages.map((pkg) => {
  const file = fs.existsSync(pkg.dir)
    ? fs.readdirSync(pkg.dir).find((f) => f.toUpperCase().startsWith('LICEN'))
    : undefined
  if (!file) {
    missing.push(pkg.name)
    return ''
  }
  const text = fs.readFileSync(path.join(pkg.dir, file), 'utf8').trim()
  return `\n## ${pkg.name} ${pkg.version}\n\n\`\`\`\n${text}\n\`\`\``
})

// ライセンス本文を同梱していないパッケージがあれば、黙って欠落させず失敗させる
// (帰属表示の漏れは配布時の権利問題に直結するため)。
if (missing.length > 0) {
  console.error(`ライセンス本文が見つかりません: ${missing.join(', ')}`)
  process.exit(1)
}

const header = [
  '# Web UI が同梱する第三者ソフトウェア',
  '',
  'ブラウザへ配信されるJavaScriptバンドルに含まれるパッケージと、その著作権表示・',
  'ライセンス全文。開発時のみ使うパッケージ(ビルドツール・テスト基盤)は配布物に',
  '含まれないため記載しない。',
  '',
  '再生成: `node scripts/gen-web-licenses.mjs`',
  '',
  '| パッケージ | バージョン | ライセンス |',
  '|---|---|---|',
  ...packages.map((p) => `| ${p.name} | ${p.version} | ${p.license} |`),
  '',
].join('\n')

const out = path.join(webDir, 'THIRD-PARTY-LICENSES.md')
fs.writeFileSync(out, header + sections.join('\n') + '\n')
console.log(`${out} を生成しました (${packages.length}パッケージ)`)
