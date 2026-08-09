//! バックグラウンドヘルスチェックタスク。APIハンドラとは独立した実行コンテキスト
//! で動き、10秒ごとに running 中の全コンテナを判定してDBへ書き戻す。
//! api/service層を経由せず repo と docker::inspector のみに依存する。

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;

use crate::docker::{ContainerRuntimeState, HealthObservation, Inspector};
use crate::domain::HealthStatus;
use crate::repo::{containers, services, Db};

const CHECK_INTERVAL: Duration = Duration::from_secs(10);
const FAILURE_THRESHOLD: u32 = 3;

pub struct HealthTask {
    db: Db,
    inspector: Inspector,
    /// container_id -> 連続失敗回数。3回連続で失敗したら unhealthy とし、1回成功で復帰する。
    /// DBには永続化しないため再起動でリセットされるが、実害は検知が最大30秒遅れる程度。
    consecutive_failures: HashMap<i64, u32>,
}

impl HealthTask {
    pub fn new(db: Db, inspector: Inspector) -> Self {
        HealthTask {
            db,
            inspector,
            consecutive_failures: HashMap::new(),
        }
    }

    pub async fn run_forever(mut self) {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = self.run_once().await {
                tracing::warn!("ヘルスチェック中にエラーが発生しました: {e}");
            }
        }
    }

    async fn run_once(&mut self) -> Result<(), sqlx::Error> {
        let rows = containers::list_for_running_services(self.db.pool()).await?;
        let now = Utc::now().to_rfc3339();

        let mut per_service: HashMap<i64, Vec<HealthStatus>> = HashMap::new();

        for row in rows {
            let container_name = sahai_core::naming::container_docker_name(row.id);
            let state = self
                .inspector
                .inspect_health(&container_name)
                .await
                .unwrap_or(ContainerRuntimeState::NotFound);

            let observed_healthy = matches!(
                state,
                ContainerRuntimeState::DockerHealth(HealthObservation::Healthy)
                    | ContainerRuntimeState::Running(true)
            );

            let health_status =
                apply_threshold(&mut self.consecutive_failures, row.id, observed_healthy);

            containers::update_health(self.db.pool(), row.id, health_status, &now).await?;
            per_service
                .entry(row.service_id)
                .or_default()
                .push(health_status);
        }

        for (service_id, statuses) in per_service {
            let worst = worst_case(&statuses);
            services::update_health_aggregate(self.db.pool(), service_id, worst, &now).await?;
        }

        Ok(())
    }
}

/// 3回連続失敗でunhealthy、1回成功でhealthyに復帰するステートマシン。
/// `HealthTask`の外に出した純粋関数(DB/Docker接続なしで直接テストできるようにするため)。
fn apply_threshold(
    failures: &mut HashMap<i64, u32>,
    container_id: i64,
    observed_healthy: bool,
) -> HealthStatus {
    if observed_healthy {
        failures.remove(&container_id);
        HealthStatus::Healthy
    } else {
        let count = failures.entry(container_id).or_insert(0);
        *count += 1;
        if *count >= FAILURE_THRESHOLD {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Unknown
        }
    }
}

fn worst_case(statuses: &[HealthStatus]) -> HealthStatus {
    if statuses.contains(&HealthStatus::Unhealthy) {
        HealthStatus::Unhealthy
    } else if statuses.iter().all(|s| *s == HealthStatus::Healthy) {
        HealthStatus::Healthy
    } else {
        HealthStatus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_and_second_consecutive_failures_are_unknown_not_unhealthy() {
        let mut failures = HashMap::new();
        assert_eq!(
            apply_threshold(&mut failures, 1, false),
            HealthStatus::Unknown
        );
        assert_eq!(
            apply_threshold(&mut failures, 1, false),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn third_consecutive_failure_becomes_unhealthy() {
        let mut failures = HashMap::new();
        apply_threshold(&mut failures, 1, false);
        apply_threshold(&mut failures, 1, false);
        assert_eq!(
            apply_threshold(&mut failures, 1, false),
            HealthStatus::Unhealthy
        );
    }

    #[test]
    fn stays_unhealthy_on_further_consecutive_failures() {
        let mut failures = HashMap::new();
        for _ in 0..5 {
            apply_threshold(&mut failures, 1, false);
        }
        assert_eq!(
            apply_threshold(&mut failures, 1, false),
            HealthStatus::Unhealthy
        );
    }

    #[test]
    fn single_success_recovers_from_unhealthy_to_healthy() {
        let mut failures = HashMap::new();
        for _ in 0..3 {
            apply_threshold(&mut failures, 1, false);
        }
        assert_eq!(
            apply_threshold(&mut failures, 1, true),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn success_resets_the_failure_counter_not_just_the_status() {
        let mut failures = HashMap::new();
        apply_threshold(&mut failures, 1, false); // 1回目失敗
        apply_threshold(&mut failures, 1, true); // 成功でリセット
                                                 // リセットされていなければここで3回目失敗扱いになりUnhealthyになってしまう
        apply_threshold(&mut failures, 1, false);
        assert_eq!(
            apply_threshold(&mut failures, 1, false),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn each_container_has_an_independent_counter() {
        let mut failures = HashMap::new();
        apply_threshold(&mut failures, 1, false);
        apply_threshold(&mut failures, 1, false);
        apply_threshold(&mut failures, 1, false); // container 1はunhealthyになる

        // container 2は初回なので影響を受けない
        assert_eq!(
            apply_threshold(&mut failures, 2, false),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn worst_case_prioritizes_unhealthy_over_everything() {
        assert_eq!(
            worst_case(&[
                HealthStatus::Healthy,
                HealthStatus::Unhealthy,
                HealthStatus::Unknown
            ]),
            HealthStatus::Unhealthy
        );
    }

    #[test]
    fn worst_case_is_healthy_only_if_all_are_healthy() {
        assert_eq!(
            worst_case(&[HealthStatus::Healthy, HealthStatus::Healthy]),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn worst_case_is_unknown_when_mixed_without_unhealthy() {
        assert_eq!(
            worst_case(&[HealthStatus::Healthy, HealthStatus::Unknown]),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn worst_case_of_empty_list_is_vacuously_healthy() {
        // Iterator::all()は空スライスに対してtrueを返すため、現状の実装では
        // コンテナ0件のサービスは"Healthy"判定になる。実際にはrun_once側で
        // per_serviceへの登録はコンテナが1件以上ある場合のみ起きるため到達しない
        // 想定だが、この関数単体としての境界挙動を明示しておく
        assert_eq!(worst_case(&[]), HealthStatus::Healthy);
    }
}
