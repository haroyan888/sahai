// APIのリクエスト/レスポンスに対応するTypeScript側の型。
// サーバー側(domain.rs・api/dto.rs)のserde表現と1対1で対応させる。
// バックエンド(crates/sahai-server/src/domain.rs, api/dto.rs)と1:1になるよう保つこと。

export type SourceType = 'image' | 'compose'
export type ServiceStatus = 'stopped' | 'running' | 'error'
export type HealthStatus = 'unknown' | 'healthy' | 'unhealthy'
export type Protocol = 'tcp' | 'udp'

export interface ServicePort {
  id: number
  container_port: number
  /** HTTP公開のポートはホストに公開しないためnull */
  host_port: number | null
  protocol: Protocol
  is_http: boolean
}

export interface ServiceVolume {
  id: number
  container_path: string
}

export interface ServiceContainer {
  id: number
  name: string
  health_status: HealthStatus
  last_health_check_at: string | null
  ports: ServicePort[]
  volumes: ServiceVolume[]
}

export interface Service {
  id: number
  name: string
  subdomain: string
  source_type: SourceType
  image: string | null
  compose_content: string | null
  env_vars: Record<string, string>
  status: ServiceStatus
  /** 起動に失敗した理由(Docker標準エラー出力)。起動成功でnullに戻る */
  last_error: string | null
  health_status: HealthStatus
  last_health_check_at: string | null
  created_at: string
  updated_at: string
}

export interface ServiceDetail extends Service {
  containers: ServiceContainer[]
  // start/restart直後、Dockerコンテナ自体は起動できたがTraefikルートの反映には
  // 失敗した場合にのみ現れる一時的な警告(domain.rs::ServiceDetail参照)。
  // 通常はキー自体が存在しない(バックエンドがskip_serializing_if済み)
  route_warning?: string
}

export interface PortInput {
  container_port: number
  /** HTTP公開のポートはホストに公開しないためnull */
  host_port: number | null
  protocol?: Protocol
  is_http?: boolean
}

export interface VolumeInput {
  container_path: string
}

export interface ContainerInput {
  name: string
  ports?: PortInput[]
  volumes?: VolumeInput[]
}

export interface CreateServiceRequest {
  name: string
  source_type: SourceType
  image?: string
  compose_content?: string
  env_vars?: Record<string, string>
  containers: ContainerInput[]
}

export interface UpdateServiceRequest {
  name?: string
  image?: string
  compose_content?: string
  env_vars?: Record<string, string>
  containers?: ContainerInput[]
}

export interface ContainerHealth {
  id: number
  name: string
  health_status: HealthStatus
  last_health_check_at: string | null
}

export interface HealthResponse {
  health_status: HealthStatus
  last_health_check_at: string | null
  containers: ContainerHealth[]
}

export interface ContainerStats {
  id: number
  name: string
  cpu_percent: number
  memory_usage_bytes: number
  memory_limit_bytes: number
}

export interface StatsResponse {
  containers: ContainerStats[]
}

export interface ContainerRegistryStatus {
  id: number
  name: string
  image_tag: string
  image_present: boolean
}

export interface RegistryStatusResponse {
  containers: ContainerRegistryStatus[]
}

// dns_provider/acme_email/registry_url/registry_username/registry_passwordはここには
// 含めない(専用のDnsConfig/RegistryConfig・「DNS/証明書設定」「レジストリ設定」画面
// でのみ変更できる。理由はsahai-server側のUpdateSettingsRequestと同じ)。
export interface Settings {
  domain: string
  https_redirect: boolean
  api_token: string
}

export interface DnsCredential {
  key: string
  value: string
}

/// DNS/証明書設定。保存するとTraefikコンテナの再作成が走る(数秒の接続断が起きる)。
export interface DnsConfig {
  dns_provider: string
  acme_email: string
  credentials: DnsCredential[]
}

// レジストリ設定(Web UI「レジストリ設定」カード)。sahai service createが
// サーバー側でdocker build/pushする際に使う資格情報+レジストリURL。
// 保存済みパスワードはGETでも平文のまま返る(DnsCredentialと同じ方針。設定画面で
// 現在値を確認・編集できるようにするため)。
export interface RegistryConfig {
  registry_url: string
  registry_username: string | null
  registry_password: string | null
  // docker loginに失敗した場合のみ含まれる(domain.rs::ServiceDetail.route_warningと
  // 同じパターン)。保存自体は成功しているため画面上は警告として表示する
  login_warning?: string
}

export interface ApiErrorField {
  field: string
  message: string
}

export interface ApiErrorBody {
  error: {
    code: string
    message: string
    fields?: ApiErrorField[]
  }
}

export interface NotServicePort {
  /** HTTP公開のポートはホストに公開しないためnull */
  host_port: number | null
  container_port: number
  protocol: Protocol
}

export interface NotServiceInfo {
  found: boolean
  name?: string
  ports?: NotServicePort[]
}

/** コンテナログの1行(SSEの`line`イベント)。 */
export interface LogLine {
  stream: 'stdout' | 'stderr'
  /** Dockerが記録した時刻。行の先頭に付いていなければnull */
  timestamp: string | null
  message: string
}
