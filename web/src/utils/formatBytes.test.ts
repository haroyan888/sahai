import { describe, expect, it } from 'vitest'
import { formatBytes } from './formatBytes'

describe('formatBytes', () => {
  it('0バイトは"0 B"と表示する', () => {
    expect(formatBytes(0)).toBe('0 B')
  })

  it('1024未満はB単位で表示する', () => {
    expect(formatBytes(512)).toBe('512 B')
  })

  it('KB単位に変換する', () => {
    expect(formatBytes(1024)).toBe('1.0 KB')
  })

  it('MB単位に変換する', () => {
    expect(formatBytes(6283264)).toBe('6.0 MB')
  })

  it('GB単位に変換する', () => {
    expect(formatBytes(8285966336)).toBe('7.7 GB')
  })
})
