// APIトークンのlocalStorage永続化の期待される振る舞いを先に定義する(TDDのRED)。

import { beforeEach, describe, expect, it } from 'vitest'
import { clearStoredToken, getStoredToken, setStoredToken } from './tokenStorage'

describe('tokenStorage', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('未保存の場合はnullを返す', () => {
    expect(getStoredToken()).toBeNull()
  })

  it('setStoredTokenで保存した値をgetStoredTokenで取得できる', () => {
    setStoredToken('my-token')
    expect(getStoredToken()).toBe('my-token')
  })

  it('clearStoredTokenで保存した値を削除する', () => {
    setStoredToken('my-token')
    clearStoredToken()
    expect(getStoredToken()).toBeNull()
  })
})
