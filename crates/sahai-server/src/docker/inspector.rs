//! bollardでのinspect/stats。source_typeに関わらず共通。

use bollard::Docker;

use super::DockerError;

/// ヘルスチェック判定の元データ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerRuntimeState {
    /// HEALTHCHECK定義があり、Dockerが判定した結果
    DockerHealth(HealthObservation),
    /// HEALTHCHECK定義がない場合のRunning状態
    Running(bool),
    /// コンテナが存在しない(未起動・削除済み等)
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthObservation {
    Healthy,
    Unhealthy,
    Starting,
}

pub struct Inspector {
    docker: Docker,
}

impl Inspector {
    pub fn new(docker: Docker) -> Self {
        Inspector { docker }
    }

    /// container_nameは"svc-{id}"形式。呼び出し側はsource_typeを意識しない。
    pub async fn inspect_health(
        &self,
        container_name: &str,
    ) -> Result<ContainerRuntimeState, DockerError> {
        let result = self
            .docker
            .inspect_container(
                container_name,
                None::<bollard::container::InspectContainerOptions>,
            )
            .await;

        let inspect = match result {
            Ok(inspect) => inspect,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(ContainerRuntimeState::NotFound),
            Err(e) => return Err(DockerError::Bollard(e)),
        };

        let state = inspect.state.unwrap_or_default();

        if let Some(health) = state.health {
            let status = health.status.map(|s| format!("{s:?}").to_lowercase());
            return Ok(ContainerRuntimeState::DockerHealth(
                match status.as_deref() {
                    Some("healthy") => HealthObservation::Healthy,
                    Some("unhealthy") => HealthObservation::Unhealthy,
                    _ => HealthObservation::Starting,
                },
            ));
        }

        Ok(ContainerRuntimeState::Running(
            state.running.unwrap_or(false),
        ))
    }

    /// `image_tag`がこのDockerホストのローカルイメージキャッシュに存在するかを確認する。
    /// レジストリのHTTP APIには直接問い合わせない(sahai-serverはレジストリの認証情報を
    /// 保持しない設計のため)。start/restart時にsahai-serverが実際に
    /// 参照するのもこの同じローカルキャッシュであるため、「起動できるかどうか」の
    /// 判定としてはこちらの方が実態に即している。
    pub async fn image_exists(&self, image_tag: &str) -> Result<bool, DockerError> {
        match self.docker.inspect_image(image_tag).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(false),
            Err(e) => Err(DockerError::Bollard(e)),
        }
    }

    pub async fn stats_once(&self, container_name: &str) -> Result<ContainerStats, DockerError> {
        use futures_util::StreamExt;

        let options = bollard::container::StatsOptions {
            stream: false,
            ..Default::default()
        };
        let mut stream = self.docker.stats(container_name, Some(options));
        let stats = stream
            .next()
            .await
            .ok_or_else(|| DockerError::Other("statsの取得に失敗しました".to_string()))??;

        let cpu_delta = stats.cpu_stats.cpu_usage.total_usage as f64
            - stats.precpu_stats.cpu_usage.total_usage as f64;
        let system_delta = stats.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
            - stats.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;
        let cpu_percent = if system_delta > 0.0 && cpu_delta > 0.0 {
            (cpu_delta / system_delta) * stats.cpu_stats.online_cpus.unwrap_or(1) as f64 * 100.0
        } else {
            0.0
        };

        Ok(ContainerStats {
            cpu_percent,
            memory_usage_bytes: stats.memory_stats.usage.unwrap_or(0),
            memory_limit_bytes: stats.memory_stats.limit.unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_limit_bytes: u64,
}
