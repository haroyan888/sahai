//! compose_content編集時のServiceContainer diff適用(core::composeを使用)。

use sqlx::SqliteConnection;

use crate::error::AppError;
use crate::repo::containers;

/// 新しい`compose_content`をパースし、既存の`ServiceContainer`とdiffを取って
/// 追加・削除を適用する。`containers`フィールドの有無とは独立して呼び出される。
/// 継続するサービスの`ServiceContainer.id`は変えない(Dockerコンテナ名が
/// `svc-{id}`のため、変えると実体のコンテナが別物になってしまう)。
pub async fn sync_containers(
    conn: &mut SqliteConnection,
    service_id: i64,
    new_compose_content: &str,
) -> Result<(), AppError> {
    let existing_rows = containers::list_by_service(&mut *conn, service_id).await?;
    let existing_names: Vec<String> = existing_rows.iter().map(|r| r.name.clone()).collect();

    let desired_names = sahai_core::compose::parse_service_names(new_compose_content)?;
    for name in &desired_names {
        sahai_core::validation::validate_compose_service_name(name)?;
    }

    let diff = sahai_core::compose::diff_container_names(&existing_names, &desired_names);

    for name in &diff.added {
        containers::insert(&mut *conn, service_id, name).await?;
    }

    for name in &diff.removed {
        if let Some(row) = existing_rows.iter().find(|r| &r.name == name) {
            containers::delete(&mut *conn, row.id).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::api::dto::{ContainerInput, CreateServiceRequest};
    use crate::repo::containers;
    use crate::service::{registration, test_support::test_state};

    use super::sync_containers;

    async fn register_webstack(state: &crate::state::AppState) -> i64 {
        let detail = registration::create(
            state,
            CreateServiceRequest {
                name: "webstack".to_string(),
                source_type: "compose".to_string(),
                image: None,
                compose_content: Some(
                    "services:\n  app:\n    build: .\n  admin:\n    image: nginx\n".to_string(),
                ),
                env_vars: None,
                containers: vec![
                    ContainerInput {
                        name: "app".to_string(),
                        ports: vec![],
                        volumes: vec![],
                    },
                    ContainerInput {
                        name: "admin".to_string(),
                        ports: vec![],
                        volumes: vec![],
                    },
                ],
            },
        )
        .await
        .unwrap();
        detail.service.id
    }

    #[tokio::test]
    async fn keeps_container_id_stable_for_unchanged_service() {
        let state = test_state().await;
        let service_id = register_webstack(&state).await;

        let before = containers::list_by_service(state.db.pool(), service_id)
            .await
            .unwrap();
        let app_id_before = before.iter().find(|c| c.name == "app").unwrap().id;

        let mut conn = state.db.pool().acquire().await.unwrap();
        sync_containers(
            &mut conn,
            service_id,
            "services:\n  app:\n    build: .\n  db:\n    image: mysql:8\n",
        )
        .await
        .unwrap();
        drop(conn);

        let after = containers::list_by_service(state.db.pool(), service_id)
            .await
            .unwrap();
        let names: Vec<&str> = after.iter().map(|c| c.name.as_str()).collect();

        assert!(names.contains(&"app"));
        assert!(names.contains(&"db"));
        assert!(
            !names.contains(&"admin"),
            "削除されたはずのadminが残っている"
        );

        let app_id_after = after.iter().find(|c| c.name == "app").unwrap().id;
        assert_eq!(
            app_id_before, app_id_after,
            "継続するコンテナのidは変わらないはず"
        );
    }

    #[tokio::test]
    async fn removed_container_cascades_its_ports() {
        let state = test_state().await;
        let service_id = register_webstack(&state).await;

        let before = containers::list_by_service(state.db.pool(), service_id)
            .await
            .unwrap();
        let admin_id = before.iter().find(|c| c.name == "admin").unwrap().id;
        crate::repo::ports::insert(
            state.db.pool(),
            admin_id,
            &crate::repo::ports::NewPort {
                container_port: 81,
                host_port: Some(20099),
                protocol: crate::domain::Protocol::Tcp,
                is_http: false,
            },
        )
        .await
        .unwrap();

        let mut conn = state.db.pool().acquire().await.unwrap();
        sync_containers(&mut conn, service_id, "services:\n  app:\n    build: .\n")
            .await
            .unwrap();
        drop(conn);

        let remaining_ports = crate::repo::ports::list_by_container(state.db.pool(), admin_id)
            .await
            .unwrap();
        assert!(remaining_ports.is_empty(), "CASCADEでports も消えるはず");
    }

    #[tokio::test]
    async fn rejects_invalid_compose_service_name() {
        let state = test_state().await;
        let service_id = register_webstack(&state).await;

        let mut conn = state.db.pool().acquire().await.unwrap();
        let result = sync_containers(
            &mut conn,
            service_id,
            "services:\n  Bad_Name!:\n    image: x\n",
        )
        .await;
        assert!(result.is_err());
    }
}
