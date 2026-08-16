//! `sahai service list/status/start/stop/restart`。Control PlaneのAPIを叩く薄いラッパー。
//! `service create`は独立したファイル(`service_create.rs`)にある
//! (アーカイブ生成・アップロードを伴う複雑な処理のため)。

use serde::Deserialize;
use serde_json::Value;

use crate::api_client::ApiClient;

pub async fn list(client: &ApiClient, json: bool) -> Result<(), String> {
    let body: Value = client.get("/api/services").await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let services = body["services"].as_array().cloned().unwrap_or_default();
    print!("{}", format_table(&services));
    Ok(())
}

/// `ServiceDetail`(サーバー側`domain::ServiceDetail`)のうち、CLI表示に使うフィールドだけを
/// 受け取るDTO。`status`(`GET /api/services/{name}`)と`start`/`stop`/`restart`
/// (`POST /api/services/{name}/(start|stop|restart)`)のレスポンスはいずれも同じ形。
#[derive(Deserialize)]
struct ServiceDetailDto {
    name: String,
    subdomain: String,
    source_type: String,
    status: String,
    health_status: String,
    last_health_check_at: Option<String>,
    #[serde(default)]
    route_warning: Option<String>,
    containers: Vec<ContainerDto>,
}

#[derive(Deserialize)]
struct ContainerDto {
    name: String,
    health_status: String,
    #[serde(default)]
    ports: Vec<PortDto>,
    #[serde(default)]
    volumes: Vec<VolumeDto>,
}

#[derive(Deserialize)]
struct PortDto {
    container_port: i64,
    host_port: i64,
    protocol: String,
    is_http: bool,
}

#[derive(Deserialize)]
struct VolumeDto {
    container_path: String,
}

/// `GET /api/services/{name}/stats`(サーバー側`StatsResponse`)のCLI表示用DTO。
#[derive(Deserialize)]
struct StatsDto {
    containers: Vec<ContainerStatsDto>,
}

#[derive(Deserialize)]
struct ContainerStatsDto {
    name: String,
    cpu_percent: f64,
    memory_usage_bytes: u64,
    memory_limit_bytes: u64,
}

/// `GET /api/services/{name}`(`ServiceDetail`)と`GET /api/services/{name}/stats`
/// (CPU/メモリ使用量)を人間可読に整形して表示する。ヘルス情報専用のエンドポイントは
/// 存在しない — `health_status`/`last_health_check_at`は`ServiceDetail`
/// (サービス全体・コンテナ別とも)に既に含まれているため。
pub async fn status(client: &ApiClient, name: &str, json: bool) -> Result<(), String> {
    let detail_raw: Value = client.get(&format!("/api/services/{name}")).await?;
    let stats_raw: Value = client.get(&format!("/api/services/{name}/stats")).await?;

    if json {
        let combined = serde_json::json!({ "service": detail_raw, "stats": stats_raw });
        println!(
            "{}",
            serde_json::to_string_pretty(&combined).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    let detail: ServiceDetailDto = serde_json::from_value(detail_raw).map_err(|e| e.to_string())?;
    let stats: StatsDto = serde_json::from_value(stats_raw).map_err(|e| e.to_string())?;
    print!("{}", format_status(&detail, &stats));
    Ok(())
}

pub async fn start(client: &ApiClient, name: &str, json: bool) -> Result<(), String> {
    let raw: Value = client
        .post_empty(&format!("/api/services/{name}/start"))
        .await?;
    print_lifecycle_result(raw, json, "起動")
}

pub async fn stop(client: &ApiClient, name: &str, json: bool) -> Result<(), String> {
    let raw: Value = client
        .post_empty(&format!("/api/services/{name}/stop"))
        .await?;
    print_lifecycle_result(raw, json, "停止")
}

pub async fn restart(client: &ApiClient, name: &str, json: bool) -> Result<(), String> {
    let raw: Value = client
        .post_empty(&format!("/api/services/{name}/restart"))
        .await?;
    print_lifecycle_result(raw, json, "再起動")
}

/// `start`/`stop`/`restart`共通の出力処理。`json`指定時は従来通り`ServiceDetail`を
/// そのままpretty printし、既定時は`format_lifecycle_result`で1行の概要+
/// (あれば)`route_warning`の警告行に整形する。
fn print_lifecycle_result(raw: Value, json: bool, action_label: &str) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&raw).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let detail: ServiceDetailDto = serde_json::from_value(raw).map_err(|e| e.to_string())?;
    print!("{}", format_lifecycle_result(action_label, &detail));
    Ok(())
}

fn format_lifecycle_result(action_label: &str, detail: &ServiceDetailDto) -> String {
    let mut out = format!(
        "サービス '{}' を{action_label}しました(status: {}, health: {})。\n",
        detail.name, detail.status, detail.health_status
    );
    if let Some(warning) = &detail.route_warning {
        out.push_str(&format!("警告: {warning}\n"));
    }
    out
}

const CONTAINER_TABLE_HEADERS: [&str; 6] = ["NAME", "HEALTH", "CPU", "MEM", "PORTS", "VOLUMES"];

/// `status`の既定(非`--json`)表示を組み立てる純粋関数(API呼び出しから分離してテストする)。
fn format_status(detail: &ServiceDetailDto, stats: &StatsDto) -> String {
    let health_suffix = detail
        .last_health_check_at
        .as_deref()
        .map(|ts| format!(" (最終チェック: {ts})"))
        .unwrap_or_default();

    let mut out = format!(
        "名前:         {}\nサブドメイン: {}\n種別:         {}\nステータス:   {}\nヘルス:       {}{health_suffix}\n",
        detail.name, detail.subdomain, detail.source_type, detail.status, detail.health_status,
    );

    if let Some(warning) = &detail.route_warning {
        out.push_str(&format!("警告: {warning}\n"));
    }

    out.push('\n');
    out.push_str("コンテナ:\n");

    let rows: Vec<Vec<String>> = detail
        .containers
        .iter()
        .map(|c| {
            let (cpu, mem) = match find_container_stats(stats, &c.name) {
                Some(s) => (
                    format!("{:.1}%", s.cpu_percent),
                    format!(
                        "{}/{}",
                        format_bytes(s.memory_usage_bytes),
                        format_bytes(s.memory_limit_bytes)
                    ),
                ),
                None => ("-".to_string(), "-".to_string()),
            };
            vec![
                c.name.clone(),
                c.health_status.clone(),
                cpu,
                mem,
                format_ports(&c.ports),
                format_volumes(&c.volumes),
            ]
        })
        .collect();

    out.push_str(&render_table(&CONTAINER_TABLE_HEADERS, &rows));
    out
}

fn find_container_stats<'a>(
    stats: &'a StatsDto,
    container_name: &str,
) -> Option<&'a ContainerStatsDto> {
    stats.containers.iter().find(|c| c.name == container_name)
}

fn format_ports(ports: &[PortDto]) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }
    ports
        .iter()
        .map(|p| {
            let suffix = if p.is_http { "(http)" } else { "" };
            format!(
                "{}->{}/{}{suffix}",
                p.host_port, p.container_port, p.protocol
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_volumes(volumes: &[VolumeDto]) -> String {
    if volumes.is_empty() {
        return "-".to_string();
    }
    volumes
        .iter()
        .map(|v| v.container_path.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

const BYTE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// `web/src/utils/formatBytes.ts`と同じアルゴリズム(1024刻み、小数点1桁)で
/// バイト数を整形する(表示単位をWeb UIと揃えるため)。
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < BYTE_UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    format!("{value:.1} {}", BYTE_UNITS[unit_index])
}

/// 任意の列数のヘッダー・行を固定幅テーブルへ整形する汎用ヘルパー(`format_table`の
/// 5列固定版とは別に、`status`のコンテナテーブル〈6列〉のために用意する)。
fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.chars().count());
            }
        }
    }

    let mut out = String::new();
    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    out.push_str(&render_table_row(&header_cells, &widths));
    for row in rows {
        out.push_str(&render_table_row(row, &widths));
    }
    out
}

fn render_table_row(cells: &[String], widths: &[usize]) -> String {
    let line: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let width = widths.get(i).copied().unwrap_or(0);
            format!("{:<width$}", c, width = width)
        })
        .collect();
    let mut line = line.join("  ");
    line.push('\n');
    line
}

const TABLE_HEADERS: [&str; 5] = ["NAME", "STATUS", "HEALTH", "TYPE", "SUBDOMAIN"];

/// `GET /api/services`の`services`配列を人間可読な固定幅テーブルへ整形する
/// (純粋関数、API呼び出しから分離してテストする)。`--json`指定時は生JSON出力になる。
fn format_table(services: &[Value]) -> String {
    if services.is_empty() {
        return "登録されているサービスはありません。\n".to_string();
    }

    let rows: Vec<[String; 5]> = services
        .iter()
        .map(|s| {
            [
                s["name"].as_str().unwrap_or("").to_string(),
                s["status"].as_str().unwrap_or("").to_string(),
                s["health_status"].as_str().unwrap_or("").to_string(),
                s["source_type"].as_str().unwrap_or("").to_string(),
                s["subdomain"].as_str().unwrap_or("").to_string(),
            ]
        })
        .collect();

    let mut widths: [usize; 5] = TABLE_HEADERS.map(str::len);
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let mut out = String::new();
    out.push_str(&format_row(&TABLE_HEADERS.map(String::from), &widths));
    for row in &rows {
        out.push_str(&format_row(row, &widths));
    }
    out
}

fn format_row(cells: &[String; 5], widths: &[usize; 5]) -> String {
    let line: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{:<width$}", c, width = widths[i]))
        .collect();
    let mut line = line.join("  ");
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_service(
        name: &str,
        status: &str,
        health: &str,
        source_type: &str,
        subdomain: &str,
    ) -> Value {
        serde_json::json!({
            "name": name,
            "status": status,
            "health_status": health,
            "source_type": source_type,
            "subdomain": subdomain,
        })
    }

    #[test]
    fn format_table_shows_placeholder_when_empty() {
        assert_eq!(format_table(&[]), "登録されているサービスはありません。\n");
    }

    #[test]
    fn format_table_includes_header_and_row_values() {
        let services = vec![sample_service(
            "myapp",
            "running",
            "healthy",
            "image",
            "myapp.example.com",
        )];
        let table = format_table(&services);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("NAME"));
        assert!(lines[1].starts_with("myapp"));
        assert!(lines[1].contains("running"));
        assert!(lines[1].contains("healthy"));
        assert!(lines[1].contains("image"));
        assert!(lines[1].contains("myapp.example.com"));
    }

    #[test]
    fn format_table_aligns_columns_by_widest_value() {
        let services = vec![
            sample_service("a", "running", "healthy", "image", "a.example.com"),
            sample_service(
                "very-long-name",
                "stopped",
                "unknown",
                "compose",
                "very-long-name.example.com",
            ),
        ];
        let table = format_table(&services);
        let lines: Vec<&str> = table.lines().collect();
        // NAME列の幅は"very-long-name"(14文字)に揃うため、"STATUS"列の開始位置が
        // 全行で一致するはず
        let header_status_pos = lines[0].find("STATUS").unwrap();
        let row1_status_pos = lines[1].find("running").unwrap();
        let row2_status_pos = lines[2].find("stopped").unwrap();
        assert_eq!(header_status_pos, row1_status_pos);
        assert_eq!(header_status_pos, row2_status_pos);
    }

    #[test]
    fn format_table_handles_missing_fields_gracefully() {
        let services = vec![serde_json::json!({ "name": "partial" })];
        let table = format_table(&services);
        assert!(table.contains("partial"));
    }

    // --- format_bytes: web/src/utils/formatBytes.tsと同じふるまいを検証 ---

    #[test]
    fn format_bytes_below_1024_uses_plain_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_exactly_1024_becomes_one_kb() {
        assert_eq!(format_bytes(1024), "1.0 KB");
    }

    #[test]
    fn format_bytes_scales_up_through_units() {
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn format_bytes_caps_at_tb_unit() {
        // TBを超える大きさでも単位はTBのまま数値だけ大きくなる(formatBytes.tsと同じ)
        let huge = 1024u64 * 1024 * 1024 * 1024 * 5;
        assert_eq!(format_bytes(huge), "5.0 TB");
    }

    // --- format_status ---

    fn detail_fixture(
        containers: Vec<ContainerDto>,
        route_warning: Option<&str>,
    ) -> ServiceDetailDto {
        ServiceDetailDto {
            name: "myapp".to_string(),
            subdomain: "myapp.example.com".to_string(),
            source_type: "image".to_string(),
            status: "running".to_string(),
            health_status: "healthy".to_string(),
            last_health_check_at: Some("2026-07-31T12:00:00.000Z".to_string()),
            route_warning: route_warning.map(str::to_string),
            containers,
        }
    }

    fn container_fixture(name: &str, ports: Vec<PortDto>, volumes: Vec<VolumeDto>) -> ContainerDto {
        ContainerDto {
            name: name.to_string(),
            health_status: "healthy".to_string(),
            ports,
            volumes,
        }
    }

    #[test]
    fn format_status_includes_header_fields() {
        let detail = detail_fixture(vec![], None);
        let stats = StatsDto { containers: vec![] };
        let out = format_status(&detail, &stats);
        assert!(out.contains("myapp"));
        assert!(out.contains("myapp.example.com"));
        assert!(out.contains("image"));
        assert!(out.contains("running"));
        assert!(out.contains("healthy"));
        assert!(out.contains("2026-07-31T12:00:00.000Z"));
    }

    #[test]
    fn format_status_omits_warning_line_when_none() {
        let detail = detail_fixture(vec![], None);
        let stats = StatsDto { containers: vec![] };
        let out = format_status(&detail, &stats);
        assert!(!out.contains("警告:"));
    }

    #[test]
    fn format_status_includes_warning_line_when_present() {
        let detail = detail_fixture(vec![], Some("Traefikルート書き出し失敗"));
        let stats = StatsDto { containers: vec![] };
        let out = format_status(&detail, &stats);
        assert!(out.contains("警告: Traefikルート書き出し失敗"));
    }

    #[test]
    fn format_status_shows_dash_for_missing_stats_and_empty_ports_volumes() {
        let detail = detail_fixture(vec![container_fixture("myapp", vec![], vec![])], None);
        let stats = StatsDto { containers: vec![] };
        let out = format_status(&detail, &stats);
        let container_line = out.lines().find(|l| l.starts_with("myapp")).unwrap();
        // CPU/MEM/PORTS/VOLUMESいずれも該当データが無いため"-"になるはず
        let dash_count = container_line.matches('-').count();
        assert!(dash_count >= 4, "{container_line}");
    }

    #[test]
    fn format_status_fills_stats_ports_and_volumes_when_present() {
        let detail = detail_fixture(
            vec![container_fixture(
                "myapp",
                vec![PortDto {
                    container_port: 80,
                    host_port: 20001,
                    protocol: "tcp".to_string(),
                    is_http: true,
                }],
                vec![VolumeDto {
                    container_path: "/var/lib/mysql".to_string(),
                }],
            )],
            None,
        );
        let stats = StatsDto {
            containers: vec![ContainerStatsDto {
                name: "myapp".to_string(),
                cpu_percent: 1.25,
                memory_usage_bytes: 1024 * 1024 * 128,
                memory_limit_bytes: 1024 * 1024 * 512,
            }],
        };
        let out = format_status(&detail, &stats);
        assert!(out.contains("1.2%"), "{out}");
        assert!(out.contains("128.0 MB/512.0 MB"), "{out}");
        assert!(out.contains("20001->80/tcp(http)"), "{out}");
        assert!(out.contains("/var/lib/mysql"), "{out}");
    }

    #[test]
    fn format_status_joins_multiple_ports_and_volumes_with_comma() {
        let detail = detail_fixture(
            vec![container_fixture(
                "app",
                vec![
                    PortDto {
                        container_port: 80,
                        host_port: 20001,
                        protocol: "tcp".to_string(),
                        is_http: true,
                    },
                    PortDto {
                        container_port: 81,
                        host_port: 20002,
                        protocol: "udp".to_string(),
                        is_http: false,
                    },
                ],
                vec![
                    VolumeDto {
                        container_path: "/data/a".to_string(),
                    },
                    VolumeDto {
                        container_path: "/data/b".to_string(),
                    },
                ],
            )],
            None,
        );
        let stats = StatsDto { containers: vec![] };
        let out = format_status(&detail, &stats);
        assert!(out.contains("20001->80/tcp(http), 20002->81/udp"), "{out}");
        assert!(out.contains("/data/a, /data/b"), "{out}");
    }

    // --- format_lifecycle_result ---

    #[test]
    fn format_lifecycle_result_summarizes_name_status_and_health() {
        let detail = detail_fixture(vec![], None);
        let out = format_lifecycle_result("起動", &detail);
        assert_eq!(
            out,
            "サービス 'myapp' を起動しました(status: running, health: healthy)。\n"
        );
    }

    #[test]
    fn format_lifecycle_result_appends_warning_line_when_present() {
        let detail = detail_fixture(vec![], Some("Traefikルート書き出し失敗"));
        let out = format_lifecycle_result("起動", &detail);
        assert!(out.contains("警告: Traefikルート書き出し失敗"));
    }

    // --- render_table(汎用テーブル整形) ---

    #[test]
    fn render_table_aligns_arbitrary_column_count() {
        let headers = ["A", "BB"];
        let rows = vec![vec!["x".to_string(), "yy".to_string()]];
        let out = render_table(&headers, &rows);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].find("BB"), lines[1].find("yy"));
    }
}
