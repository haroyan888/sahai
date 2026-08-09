import { describe, expect, it } from 'vitest'
import { mergeEnvVarRows, parseEnvFile } from './parseEnvFile'

describe('parseEnvFile', () => {
  it('キーと値のペアを読み取る', () => {
    expect(parseEnvFile('FOO=bar\nBAZ=qux\n')).toEqual([
      { key: 'FOO', value: 'bar' },
      { key: 'BAZ', value: 'qux' },
    ])
  })

  it('コメント行と空行を無視する', () => {
    expect(parseEnvFile('# コメント\n\nFOO=bar\n   \n# 末尾コメント')).toEqual([{ key: 'FOO', value: 'bar' }])
  })

  it('値に含まれる=は分割せず値の一部として扱う', () => {
    expect(parseEnvFile('DATABASE_URL=postgres://u:p@h/db?a=1')).toEqual([
      { key: 'DATABASE_URL', value: 'postgres://u:p@h/db?a=1' },
    ])
  })

  it('export接頭辞を取り除く', () => {
    expect(parseEnvFile('export FOO=bar')).toEqual([{ key: 'FOO', value: 'bar' }])
  })

  it('値を囲む引用符を取り除く', () => {
    expect(parseEnvFile('A="hello world"\nB=\'single\'')).toEqual([
      { key: 'A', value: 'hello world' },
      { key: 'B', value: 'single' },
    ])
  })

  it('キー・値の前後の空白を取り除く', () => {
    expect(parseEnvFile('  FOO = bar  ')).toEqual([{ key: 'FOO', value: 'bar' }])
  })

  it('空の値を許容する', () => {
    expect(parseEnvFile('EMPTY=')).toEqual([{ key: 'EMPTY', value: '' }])
  })

  it('=を持たない行や不正なキーの行は無視する', () => {
    expect(parseEnvFile('これは説明文です\n=値だけ\n1BAD=x\nBAD KEY=x\nGOOD=y')).toEqual([
      { key: 'GOOD', value: 'y' },
    ])
  })

  it('CRLF改行を扱える', () => {
    expect(parseEnvFile('FOO=bar\r\nBAZ=qux')).toEqual([
      { key: 'FOO', value: 'bar' },
      { key: 'BAZ', value: 'qux' },
    ])
  })
})

describe('mergeEnvVarRows', () => {
  it('同じキーは上書きし、無いキーは追加する', () => {
    const existing = [
      { key: 'KEEP', value: '1' },
      { key: 'OVERWRITE', value: 'old' },
    ]
    const loaded = [
      { key: 'OVERWRITE', value: 'new' },
      { key: 'ADDED', value: '2' },
    ]
    expect(mergeEnvVarRows(existing, loaded)).toEqual([
      { key: 'KEEP', value: '1' },
      { key: 'OVERWRITE', value: 'new' },
      { key: 'ADDED', value: '2' },
    ])
  })

  it('既存が空でも読み込んだ内容がそのまま入る', () => {
    expect(mergeEnvVarRows([], [{ key: 'A', value: '1' }])).toEqual([{ key: 'A', value: '1' }])
  })

  it('元の配列を破壊しない', () => {
    const existing = [{ key: 'A', value: '1' }]
    mergeEnvVarRows(existing, [{ key: 'A', value: '2' }])
    expect(existing).toEqual([{ key: 'A', value: '1' }])
  })
})
