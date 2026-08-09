//! start/stop/restartのオーケストレーション、冪等性判定。

use crate::domain::{ServiceDetail, ServiceStatus};
use crate::error::AppError;
use crate::repo::services;
use crate::state::AppState;

/// 既にrunning中なら何もせず現在の状態を返す真の冪等no-op。
pub async fn start(state: &AppState, id_or_name: &str) -> Result<ServiceDetail, AppError> {
    let detail = super::load_detail(state, id_or_name).await?;

    if detail.service.status == ServiceStatus::Running {
        return Ok(detail);
    }

    let runtime = state.docker.runtime_for(detail.service.source_type);
    let result = runtime.start(&detail).await;

    let new_status = match &result {
        Ok(()) => ServiceStatus::Running,
        Err(e) => {
            tracing::warn!("起動に失敗しました(service_id={}): {e}", detail.service.id);
            ServiceStatus::Error
        }
    };
    services::update_status(state.db.pool(), detail.service.id, new_status).await?;

    let mut updated = super::load_detail_by_id(state, detail.service.id).await?;
    if let Err(e) = state.traefik.write_route(&updated).await {
        // Dockerコンテナ自体の起動は成功しているため、ここで200を50xに変えて
        // 呼び出し元を混乱させない(既にstatus=runningのサービスへの/startは
        // 冪等no-opで何もしないため、エラーにすると「再実行しても直らない」
        // 詰み状態を作ってしまう)。代わりにroute_warningへ理由を積んで呼び出し元へ
        // 確実に伝える。
        let msg = format!(
            "Traefikルートの反映に失敗しました: {e}。もう一度反映するには再起動(restart)をお試しください"
        );
        tracing::error!("service_id={}: {msg}", detail.service.id);
        updated.route_warning = Some(msg);
    }

    Ok(updated)
}

/// 既にstopped中なら何もせず現在の状態を返す冪等no-op。
pub async fn stop(state: &AppState, id_or_name: &str) -> Result<ServiceDetail, AppError> {
    let detail = super::load_detail(state, id_or_name).await?;

    if detail.service.status == ServiceStatus::Stopped {
        return Ok(detail);
    }

    let runtime = state.docker.runtime_for(detail.service.source_type);
    runtime
        .stop(&detail)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    services::update_status(state.db.pool(), detail.service.id, ServiceStatus::Stopped).await?;

    super::load_detail_by_id(state, detail.service.id).await
}

/// stop→start(7章)。stop後は必ずstopped状態のため、startのno-op分岐には入らない。
pub async fn restart(state: &AppState, id_or_name: &str) -> Result<ServiceDetail, AppError> {
    stop(state, id_or_name).await?;
    start(state, id_or_name).await
}

#[cfg(test)]
mod tests {
    use crate::api::dto::{ContainerInput, CreateServiceRequest, PortInput};
    use crate::service::{registration, test_support::test_state};

    use super::*;

    /// 実Dockerデーモンに対する結合テスト。`cargo test -- --ignored`で明示的に実行する。
    /// 登録→起動(実コンテナ作成)→DB上のstatus確認→実コンテナのRunning確認→停止、を
    /// service層のAPIだけを使ってフルスタックで検証する。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn e2e_start_and_stop_through_service_layer() {
        let state = test_state().await;
        let detail = registration::create(
            &state,
            CreateServiceRequest {
                name: "e2eweb".to_string(),
                source_type: "image".to_string(),
                image: Some("nginx:alpine".to_string()),
                compose_content: None,
                env_vars: None,
                containers: vec![ContainerInput {
                    name: "e2eweb".to_string(),
                    ports: vec![PortInput {
                        container_port: 80,
                        host_port: 21101,
                        protocol: "tcp".to_string(),
                        is_http: true,
                    }],
                    volumes: vec![],
                }],
            },
        )
        .await
        .unwrap();

        let started = start(&state, "e2eweb").await.unwrap();

        let inspect_result = bollard::Docker::connect_with_local_defaults()
            .unwrap()
            .inspect_container(
                &format!("svc-{}", detail.containers[0].container.id),
                None::<bollard::container::InspectContainerOptions>,
            )
            .await;

        let stopped = stop(&state, "e2eweb").await;

        assert_eq!(
            started.service.status,
            crate::domain::ServiceStatus::Running,
            "起動後はDB上もrunningになるはず"
        );
        let inspect = inspect_result.expect("実コンテナをinspectできるはず");
        assert_eq!(inspect.state.and_then(|s| s.running), Some(true));

        let stopped = stopped.unwrap();
        assert_eq!(
            stopped.service.status,
            crate::domain::ServiceStatus::Stopped
        );
    }

    /// 実Dockerデーモンに対する結合テスト。`cargo test -- --ignored`で明示的に実行する。
    /// Dockerコンテナ自体は正常に起動するが、Traefikルートの書き出し先
    /// (`traefik_dynamic_dir`)を意図的にファイルで塞いでおき、書き出し失敗が
    /// サイレントに握りつぶされず`route_warning`として返ることを検証する。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn e2e_start_surfaces_warning_when_traefik_route_write_fails() {
        let state = test_state().await;

        let dynamic_dir = state.config.traefik_dynamic_dir();
        tokio::fs::create_dir_all(dynamic_dir.parent().unwrap())
            .await
            .unwrap();
        // dynamic_dir自体をファイルにしておくと、write_route内のcreate_dir_allが
        // 「既存の非ディレクトリを掘り下げようとする」形で失敗する
        tokio::fs::write(&dynamic_dir, b"blocker").await.unwrap();

        registration::create(
            &state,
            CreateServiceRequest {
                name: "e2ewarn".to_string(),
                source_type: "image".to_string(),
                image: Some("nginx:alpine".to_string()),
                compose_content: None,
                env_vars: None,
                containers: vec![ContainerInput {
                    name: "e2ewarn".to_string(),
                    ports: vec![PortInput {
                        container_port: 80,
                        host_port: 21102,
                        protocol: "tcp".to_string(),
                        is_http: true,
                    }],
                    volumes: vec![],
                }],
            },
        )
        .await
        .unwrap();

        let started = start(&state, "e2ewarn").await.unwrap();

        assert_eq!(
            started.service.status,
            crate::domain::ServiceStatus::Running,
            "Dockerコンテナ自体は正常に起動しているはず"
        );
        assert!(
            started.route_warning.is_some(),
            "Traefikルート書き出し失敗がroute_warningに反映されるはず"
        );

        // 後始末: dynamic_dirを塞いだままだとstopできないため元に戻してから停止する
        tokio::fs::remove_file(&dynamic_dir).await.unwrap();
        let _ = stop(&state, "e2ewarn").await;
    }
}
