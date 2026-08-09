// Control Plane APIを叩くクライアント。

import type {
  ApiErrorBody,
  ApiErrorField,
  CreateServiceRequest,
  DnsConfig,
  HealthResponse,
  RegistryConfig,
  RegistryStatusResponse,
  Service,
  ServiceDetail,
  Settings,
  StatsResponse,
  UpdateServiceRequest,
} from './types'

export interface ApiClientConfig {
  baseUrl: string
  token: string
  /**
   * 401が返ったときに呼ばれる。トークンが無効・変更された場合に、各画面が
   * それぞれ「取得に失敗しました」と表示して行き止まりになるのを防ぎ、
   * 呼び出し側(App)がログイン画面へ戻す。
   */
  onUnauthorized?: () => void
}

/** APIのエラーレスポンス({ error: { code, message, fields } })をラップする例外。 */
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly fields?: ApiErrorBody['error']['fields']

  constructor(status: number, code: string, message: string, fields?: ApiErrorBody['error']['fields']) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.fields = fields
  }
}

/**
 * catchしたエラーがApiErrorであれば`fields`/`message`を取り出す。各フォーム画面で
 * `if (err instanceof ApiError) { setFieldErrors(...); setError(...) } else { ... }`
 * という同じ分岐が繰り返されていたため抽出した。ApiErrorでなければnullを返すので、
 * 呼び出し側は好きなフォールバックメッセージを出し分けられる。
 */
export function parseApiError(err: unknown): { fields: ApiErrorField[]; message: string } | null {
  if (!(err instanceof ApiError)) return null
  return { fields: err.fields ?? [], message: err.message }
}

export interface ApiClient {
  listServices(): Promise<Service[]>
  getService(idOrName: string): Promise<ServiceDetail>
  createService(req: CreateServiceRequest): Promise<ServiceDetail>
  updateService(idOrName: string, req: UpdateServiceRequest): Promise<ServiceDetail>
  deleteService(idOrName: string, purgeVolumes?: boolean): Promise<void>
  startService(idOrName: string): Promise<ServiceDetail>
  stopService(idOrName: string): Promise<ServiceDetail>
  restartService(idOrName: string): Promise<ServiceDetail>
  getHealth(idOrName: string): Promise<HealthResponse>
  getStats(idOrName: string): Promise<StatsResponse>
  getRegistryStatus(idOrName: string): Promise<RegistryStatusResponse>
  getSettings(): Promise<Settings>
  updateSettings(settings: Settings): Promise<Settings>
  getDnsConfig(): Promise<DnsConfig>
  updateDnsConfig(config: DnsConfig): Promise<DnsConfig>
  getRegistryConfig(): Promise<RegistryConfig>
  updateRegistryConfig(config: RegistryConfig): Promise<RegistryConfig>
}

/** 雛形のみ。実装は後続のGREENフェーズで行う。 */
export function createApiClient(config: ApiClientConfig): ApiClient {
  const base = config.baseUrl.replace(/\/$/, '')

  async function request<T>(path: string, init?: RequestInit): Promise<T | undefined> {
    const response = await fetch(`${base}${path}`, {
      ...init,
      headers: {
        Authorization: `Bearer ${config.token}`,
        ...init?.headers,
      },
    })

    if (!response.ok) {
      if (response.status === 401) {
        config.onUnauthorized?.()
      }
      const body = (await response.json()) as ApiErrorBody
      throw new ApiError(response.status, body.error.code, body.error.message, body.error.fields)
    }

    if (response.status === 204) {
      return undefined
    }

    return (await response.json()) as T
  }

  function jsonInit(method: string, body: unknown): RequestInit {
    return {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }
  }

  return {
    async listServices() {
      const data = await request<{ services: Service[] }>('/api/services', { method: 'GET' })
      return data!.services
    },
    async getService(idOrName: string) {
      return (await request<ServiceDetail>(`/api/services/${idOrName}`, { method: 'GET' }))!
    },
    async createService(req: CreateServiceRequest) {
      return (await request<ServiceDetail>('/api/services', jsonInit('POST', req)))!
    },
    async updateService(idOrName: string, req: UpdateServiceRequest) {
      return (await request<ServiceDetail>(`/api/services/${idOrName}`, jsonInit('PUT', req)))!
    },
    async deleteService(idOrName: string, purgeVolumes = false) {
      await request<void>(`/api/services/${idOrName}?purge_volumes=${purgeVolumes}`, { method: 'DELETE' })
    },
    async startService(idOrName: string) {
      return (await request<ServiceDetail>(`/api/services/${idOrName}/start`, { method: 'POST' }))!
    },
    async stopService(idOrName: string) {
      return (await request<ServiceDetail>(`/api/services/${idOrName}/stop`, { method: 'POST' }))!
    },
    async restartService(idOrName: string) {
      return (await request<ServiceDetail>(`/api/services/${idOrName}/restart`, { method: 'POST' }))!
    },
    async getHealth(idOrName: string) {
      return (await request<HealthResponse>(`/api/services/${idOrName}/health`, { method: 'GET' }))!
    },
    async getStats(idOrName: string) {
      return (await request<StatsResponse>(`/api/services/${idOrName}/stats`, { method: 'GET' }))!
    },
    async getRegistryStatus(idOrName: string) {
      return (await request<RegistryStatusResponse>(`/api/services/${idOrName}/registry`, { method: 'GET' }))!
    },
    async getSettings() {
      return (await request<Settings>('/api/settings', { method: 'GET' }))!
    },
    async updateSettings(settings: Settings) {
      return (await request<Settings>('/api/settings', jsonInit('PUT', settings)))!
    },
    async getDnsConfig() {
      return (await request<DnsConfig>('/api/settings/dns-provider', { method: 'GET' }))!
    },
    async updateDnsConfig(config: DnsConfig) {
      return (await request<DnsConfig>('/api/settings/dns-provider', jsonInit('PUT', config)))!
    },
    async getRegistryConfig() {
      return (await request<RegistryConfig>('/api/settings/registry', { method: 'GET' }))!
    },
    async updateRegistryConfig(config: RegistryConfig) {
      return (await request<RegistryConfig>('/api/settings/registry', jsonInit('PUT', config)))!
    },
  }
}
