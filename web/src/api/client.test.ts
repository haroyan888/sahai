// createApiClient()の期待される振る舞いを先に定義する(TDDのRED)。
// 現時点ではclient.tsの各関数は`not implemented`をthrowするだけなので、
// このファイルのテストはすべて失敗する想定(GREENフェーズで実装する)。

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError, createApiClient, parseApiError } from './client'
import type { ServiceDetail } from './types'

const BASE_URL = 'https://admin.example.com'
const TOKEN = 'test-token'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function sampleServiceDetail(overrides: Partial<ServiceDetail> = {}): ServiceDetail {
  return {
    id: 1,
    name: 'myapp',
    subdomain: 'myapp.example.com',
    source_type: 'image',
    image: 'registry.sahai.example.com/myapp:latest',
    compose_content: null,
    env_vars: {},
    status: 'stopped',
    last_error: null,
    health_status: 'unknown',
    last_health_check_at: null,
    created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z',
    containers: [],
    ...overrides,
  }
}

describe('createApiClient', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn())
  })

  it('listServices: GETリクエストをBearer認証付きで送り、servicesの配列を返す', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ services: [sampleServiceDetail()] }))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const services = await client.listServices()

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services`,
      expect.objectContaining({
        method: 'GET',
        headers: expect.objectContaining({ Authorization: `Bearer ${TOKEN}` }),
      }),
    )
    expect(services).toHaveLength(1)
    expect(services[0].name).toBe('myapp')
  })

  it('getService: {id_or_name}をパスに埋め込んでGETする', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse(sampleServiceDetail()))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const detail = await client.getService('myapp')

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp`,
      expect.objectContaining({ method: 'GET' }),
    )
    expect(detail.name).toBe('myapp')
  })

  it('createService: POST /api/services にJSONボディを送り201のレスポンスを返す', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse(sampleServiceDetail(), 201))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const req = {
      name: 'myapp',
      source_type: 'image' as const,
      image: 'x:latest',
      containers: [{ name: 'myapp' }],
    }
    await client.createService(req)

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services`,
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      }),
    )
  })

  it('updateService: PUT /api/services/{id_or_name} にJSONボディを送る', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse(sampleServiceDetail({ name: 'renamed' })))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.updateService('myapp', { name: 'renamed' })

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp`,
      expect.objectContaining({
        method: 'PUT',
        body: JSON.stringify({ name: 'renamed' }),
      }),
    )
  })

  it('deleteService: DELETEし、purgeVolumes未指定時はpurge_volumes=falseをクエリに付ける', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.deleteService('myapp')

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp?purge_volumes=false`,
      expect.objectContaining({ method: 'DELETE' }),
    )
  })

  it('deleteService: purgeVolumes=trueを渡すとクエリに反映される', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.deleteService('myapp', true)

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp?purge_volumes=true`,
      expect.objectContaining({ method: 'DELETE' }),
    )
  })

  it.each(['startService', 'stopService', 'restartService'] as const)(
    '%s: POST /api/services/{id_or_name}/<action> を空ボディで送る',
    async (method) => {
      const fetchMock = vi.mocked(fetch)
      fetchMock.mockResolvedValueOnce(jsonResponse(sampleServiceDetail({ status: 'running' })))

      const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
      await client[method]('myapp')

      const action = method.replace('Service', '').toLowerCase()
      expect(fetchMock).toHaveBeenCalledWith(
        `${BASE_URL}/api/services/myapp/${action}`,
        expect.objectContaining({ method: 'POST' }),
      )
    },
  )

  it('getHealth: GET /api/services/{id_or_name}/health を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ health_status: 'healthy', last_health_check_at: null, containers: [] }),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const health = await client.getHealth('myapp')

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp/health`,
      expect.objectContaining({ method: 'GET' }),
    )
    expect(health.health_status).toBe('healthy')
  })

  it('getStats: GET /api/services/{id_or_name}/stats を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ containers: [] }))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.getStats('myapp')

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp/stats`,
      expect.objectContaining({ method: 'GET' }),
    )
  })

  it('getRegistryStatus: GET /api/services/{id_or_name}/registry を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ containers: [] }))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.getRegistryStatus('myapp')

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/services/myapp/registry`,
      expect.objectContaining({ method: 'GET' }),
    )
  })

  it('getSettings: GET /api/settings を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        domain: 'example.com',
        https_redirect: true,
        api_token: 'tok',
      }),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const settings = await client.getSettings()

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings`,
      expect.objectContaining({ method: 'GET' }),
    )
    expect(settings.domain).toBe('example.com')
  })

  it('updateSettings: PUT /api/settings を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    const settings = {
      domain: 'example.com',
      https_redirect: false,
      api_token: 'newtok',
    }
    fetchMock.mockResolvedValueOnce(jsonResponse(settings))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.updateSettings(settings)

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings`,
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(settings) }),
    )
  })

  it('getRegistryConfig: GET /api/settings/registry を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        registry_url: 'registry.sahai.example.com',
        registry_username: 'reguser',
        registry_password: 'regpass',
      }),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const config = await client.getRegistryConfig()

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings/registry`,
      expect.objectContaining({ method: 'GET' }),
    )
    expect(config.registry_url).toBe('registry.sahai.example.com')
    expect(config.registry_username).toBe('reguser')
  })

  it('updateRegistryConfig: PUT /api/settings/registry を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    const config = {
      registry_url: 'registry.sahai.example.com',
      registry_username: 'reguser',
      registry_password: 'regpass',
    }
    fetchMock.mockResolvedValueOnce(jsonResponse(config))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.updateRegistryConfig(config)

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings/registry`,
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(config) }),
    )
  })

  it('getDnsConfig: GET /api/settings/dns-provider を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        dns_provider: 'cloudflare',
        acme_email: 'admin@example.com',
        credentials: [{ key: 'CF_DNS_API_TOKEN', value: 'secret' }],
      }),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    const config = await client.getDnsConfig()

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings/dns-provider`,
      expect.objectContaining({ method: 'GET' }),
    )
    expect(config.dns_provider).toBe('cloudflare')
    expect(config.credentials).toEqual([{ key: 'CF_DNS_API_TOKEN', value: 'secret' }])
  })

  it('updateDnsConfig: PUT /api/settings/dns-provider を叩く', async () => {
    const fetchMock = vi.mocked(fetch)
    const config = {
      dns_provider: 'route53',
      acme_email: 'admin@example.com',
      credentials: [{ key: 'AWS_ACCESS_KEY_ID', value: 'AKIA...' }],
    }
    fetchMock.mockResolvedValueOnce(jsonResponse(config))

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })
    await client.updateDnsConfig(config)

    expect(fetchMock).toHaveBeenCalledWith(
      `${BASE_URL}/api/settings/dns-provider`,
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(config) }),
    )
  })

  it('baseUrlの末尾スラッシュを二重にしない', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(jsonResponse({ services: [] }))

    const client = createApiClient({ baseUrl: `${BASE_URL}/`, token: TOKEN })
    await client.listServices()

    expect(fetchMock).toHaveBeenCalledWith(`${BASE_URL}/api/services`, expect.anything())
  })

  it('エラーレスポンス(4xx/5xx)はApiErrorとしてthrowされ、code/message/fieldsを保持する', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse(
        {
          error: {
            code: 'VALIDATION_ERROR',
            message: '入力内容に誤りがあります',
            fields: [{ field: 'name', message: 'サービス名は英小文字で始まる必要があります' }],
          },
        },
        400,
      ),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })

    await expect(
      client.createService({
        name: 'BadName',
        source_type: 'image',
        containers: [],
      }),
    ).rejects.toMatchObject(
      expect.objectContaining({
        status: 400,
        code: 'VALIDATION_ERROR',
        fields: [{ field: 'name', message: 'サービス名は英小文字で始まる必要があります' }],
      }),
    )
  })

  it('ApiErrorはError/ApiErrorのinstanceofとして判定できる', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ error: { code: 'NOT_FOUND', message: '見つかりません' } }, 404),
    )

    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })

    try {
      await client.getService('doesnotexist')
      expect.unreachable('エラーがthrowされるはず')
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError)
      expect(e).toBeInstanceOf(Error)
    }
  })
})

describe('parseApiError', () => {
  it('ApiErrorならfields(既定値[])とmessageを取り出す', () => {
    const err = new ApiError(400, 'VALIDATION_ERROR', '入力内容に誤りがあります', [
      { field: 'domain', message: 'ドメインを入力してください' },
    ])
    expect(parseApiError(err)).toEqual({
      fields: [{ field: 'domain', message: 'ドメインを入力してください' }],
      message: '入力内容に誤りがあります',
    })
  })

  it('fieldsを持たないApiErrorは空配列を返す', () => {
    const err = new ApiError(404, 'NOT_FOUND', '見つかりません')
    expect(parseApiError(err)).toEqual({ fields: [], message: '見つかりません' })
  })

  it('ApiError以外はnullを返す', () => {
    expect(parseApiError(new Error('network error'))).toBeNull()
    expect(parseApiError('not an error')).toBeNull()
  })
})

describe('401時の扱い', () => {
  it('401を受けたらonUnauthorizedを呼び、ApiErrorもthrowする', async () => {
    const onUnauthorized = vi.fn()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse({ error: { code: 'UNAUTHORIZED', message: '認証が必要です' } }, 401),
      ),
    )
    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN, onUnauthorized })

    await expect(client.listServices()).rejects.toBeInstanceOf(ApiError)
    expect(onUnauthorized).toHaveBeenCalledTimes(1)
  })

  it('401以外のエラーではonUnauthorizedを呼ばない', async () => {
    const onUnauthorized = vi.fn()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse({ error: { code: 'NOT_FOUND', message: '見つかりません' } }, 404),
      ),
    )
    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN, onUnauthorized })

    await expect(client.listServices()).rejects.toBeInstanceOf(ApiError)
    expect(onUnauthorized).not.toHaveBeenCalled()
  })

  it('onUnauthorizedを渡さなくても例外にならない', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse({ error: { code: 'UNAUTHORIZED', message: '認証が必要です' } }, 401),
      ),
    )
    const client = createApiClient({ baseUrl: BASE_URL, token: TOKEN })

    await expect(client.listServices()).rejects.toBeInstanceOf(ApiError)
  })
})
