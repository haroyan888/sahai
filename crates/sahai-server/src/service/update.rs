//! PUT: nameの即時反映とその他フィールドの遅延反映を分岐。

use sahai_core::validation;

use crate::api::dto::UpdateServiceRequest;
use crate::domain::{Protocol, ServiceDetail};
use crate::error::{AppError, FieldError};
use crate::repo::{containers, ports, services, volumes, ImmediateTransaction};
use crate::state::AppState;

pub async fn update(
    state: &AppState,
    id_or_name: &str,
    req: UpdateServiceRequest,
) -> Result<ServiceDetail, AppError> {
    let current = super::load_detail(state, id_or_name).await?;
    let service_id = current.service.id;

    // --- name: 即時反映(稼働中でも可。コンテナ名はsvc-{id}でnameに依存しないため) ---
    if let Some(new_name) = &req.name {
        validation::validate_service_name(new_name)
            .map_err(|e| AppError::validation_single("name", e.to_string()))?;

        let old_subdomain = current.service.subdomain.clone();
        let domain = state.settings.read().await.domain.clone();
        let new_subdomain = sahai_core::naming::subdomain_for(new_name, &domain);
        services::update_name(state.db.pool(), service_id, new_name, &new_subdomain).await?;

        if current.service.source_type == crate::domain::SourceType::Image {
            if let Some(only_container) = current.containers.first() {
                containers::update_name(state.db.pool(), only_container.container.id, new_name)
                    .await?;
            }
        }

        let updated = super::load_detail_by_id(state, service_id).await?;
        state
            .traefik
            .remove_route(&old_subdomain)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        state
            .traefik
            .write_route(&updated)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // --- 以下は保存のみ。反映は次回start/restart時 ---
    let mut tx = ImmediateTransaction::begin(&state.db).await?;

    let result = apply_deferred_updates(tx.conn(), service_id, &req).await;

    match result {
        Ok(()) => tx.commit().await?,
        Err(e) => {
            // BEGIN IMMEDIATEしたコネクションをcommit/rollbackせずに手放すと、
            // プールに「トランザクション開始済み」のまま返却され、以降の全操作が
            // 壊れる(registration.rsの同種の修正・テスト参照)
            let _ = tx.rollback().await;
            return Err(e);
        }
    }

    super::load_detail_by_id(state, service_id).await
}

async fn apply_deferred_updates(
    conn: &mut sqlx::SqliteConnection,
    service_id: i64,
    req: &UpdateServiceRequest,
) -> Result<(), AppError> {
    if let Some(env_vars) = &req.env_vars {
        services::update_env_vars(&mut *conn, service_id, env_vars).await?;
    }
    if let Some(image) = &req.image {
        services::update_image(&mut *conn, service_id, image).await?;
    }

    // compose_contentが指定されていればdiffを実行する。containersの有無とは独立
    if let Some(compose_content) = &req.compose_content {
        super::compose_sync::sync_containers(&mut *conn, service_id, compose_content).await?;
        services::update_compose_content(&mut *conn, service_id, compose_content).await?;
    }

    if let Some(container_inputs) = &req.containers {
        apply_container_updates(&mut *conn, service_id, container_inputs).await?;
    }

    Ok(())
}

async fn apply_container_updates(
    conn: &mut sqlx::SqliteConnection,
    service_id: i64,
    inputs: &[crate::api::dto::ContainerInput],
) -> Result<(), AppError> {
    let existing = containers::list_by_service(&mut *conn, service_id).await?;

    let mut errors = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        if !existing.iter().any(|c| c.name == input.name) {
            errors.push(FieldError {
                field: format!("containers[{i}].name"),
                message: format!(
                    "'{}' は現時点で実在するコンテナ名ではありません",
                    input.name
                ),
            });
        }
    }
    errors.extend(super::port_check::collect_request_errors(inputs));
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }

    // 自サービスのポートは除外する。既存の値をそのまま保存し直すのは衝突ではない
    // (この時点ではまだ古い行が残っているため、除外しないと必ず自分自身と衝突する)。
    super::port_check::check_against_existing(&mut *conn, inputs, Some(service_id)).await?;

    for (i, input) in inputs.iter().enumerate() {
        let Some(target) = existing.iter().find(|c| c.name == input.name) else {
            continue;
        };

        ports::delete_by_container(&mut *conn, target.id).await?;
        volumes::delete_by_container(&mut *conn, target.id).await?;

        for (j, p) in input.ports.iter().enumerate() {
            let protocol = Protocol::try_from(p.protocol.as_str()).map_err(|e| {
                AppError::validation_single(format!("containers[{i}].ports[{j}].protocol"), e)
            })?;
            ports::insert(
                &mut *conn,
                target.id,
                &ports::NewPort {
                    container_port: p.container_port,
                    host_port: p.host_port,
                    protocol,
                    is_http: p.is_http,
                },
            )
            .await?;
        }
        for v in &input.volumes {
            volumes::insert(&mut *conn, target.id, &v.container_path).await?;
        }
    }

    // is_httpはサービスにつき最大1件であることを、置き換え適用後の最終状態に対して
    // 検証する。DBのUNIQUE INDEXは同一コンテナ内の重複しか
    // 防げないため、コンテナを横断するこのチェックはアプリ層の責務(11章)。
    let http_count = ports::count_http_ports_for_service(&mut *conn, service_id).await?;
    if http_count > 1 {
        return Err(AppError::validation_single(
            "containers[].ports[].is_http",
            "is_httpはサービスにつき最大1件までです",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::api::dto::{ContainerInput, CreateServiceRequest, PortInput, UpdateServiceRequest};
    use crate::error::AppError;
    use crate::service::{registration, test_support::test_state, update::update};

    fn compose_request() -> CreateServiceRequest {
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
                    ports: vec![PortInput {
                        container_port: 80,
                        host_port: 20020,
                        protocol: "tcp".to_string(),
                        is_http: true,
                    }],
                    volumes: vec![],
                },
                ContainerInput {
                    name: "admin".to_string(),
                    ports: vec![PortInput {
                        container_port: 81,
                        host_port: 20021,
                        protocol: "tcp".to_string(),
                        is_http: false,
                    }],
                    volumes: vec![],
                },
            ],
        }
    }

    // RED: PUTで別コンテナにis_httpを追加し、サービス全体でis_http=2件になるケースは
    // 拒否されるべき(is_httpがサービスにつき最大1件であることの
    // チェックは、上記1・2適用後の最終状態に対して行う」)。
    #[tokio::test]
    async fn update_rejects_second_http_port_across_containers() {
        let state = test_state().await;
        registration::create(&state, compose_request())
            .await
            .unwrap();

        let result = update(
            &state,
            "webstack",
            UpdateServiceRequest {
                containers: Some(vec![ContainerInput {
                    name: "admin".to_string(),
                    ports: vec![PortInput {
                        container_port: 81,
                        host_port: 20021,
                        protocol: "tcp".to_string(),
                        is_http: true,
                    }],
                    volumes: vec![],
                }]),
                ..Default::default()
            },
        )
        .await;

        assert!(
            matches!(result, Err(AppError::Validation(_))),
            "is_httpが2件になる更新はVALIDATION_ERRORで拒否されるべき: {result:?}"
        );
    }

    #[tokio::test]
    async fn update_allows_moving_the_only_http_port_to_another_container() {
        let state = test_state().await;
        registration::create(&state, compose_request())
            .await
            .unwrap();

        // appからis_httpを外し、adminへ付け替える(全体では常に1件)
        let result = update(
            &state,
            "webstack",
            UpdateServiceRequest {
                containers: Some(vec![
                    ContainerInput {
                        name: "app".to_string(),
                        ports: vec![PortInput {
                            container_port: 80,
                            host_port: 20020,
                            protocol: "tcp".to_string(),
                            is_http: false,
                        }],
                        volumes: vec![],
                    },
                    ContainerInput {
                        name: "admin".to_string(),
                        ports: vec![PortInput {
                            container_port: 81,
                            host_port: 20021,
                            protocol: "tcp".to_string(),
                            is_http: true,
                        }],
                        volumes: vec![],
                    },
                ]),
                ..Default::default()
            },
        )
        .await;

        assert!(
            result.is_ok(),
            "is_http合計1件の付け替えは許可されるべき: {result:?}"
        );
    }

    // "sahai"/"registry"への名前変更は管理画面・レジストリの静的Traefikルートと
    // 衝突するため拒否されるべき(sahai-core::validation::RESERVED_SERVICE_NAMES参照)。
    #[tokio::test]
    async fn update_rejects_renaming_to_a_reserved_name() {
        let state = test_state().await;
        registration::create(&state, compose_request())
            .await
            .unwrap();

        let result = update(
            &state,
            "webstack",
            UpdateServiceRequest {
                name: Some("sahai".to_string()),
                ..Default::default()
            },
        )
        .await;

        match result {
            Err(AppError::Validation(fields)) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].field, "name");
            }
            other => panic!("VALIDATION_ERRORかつnameフィールドを期待: {other:?}"),
        }
    }

    // 存在しないコンテナ名を指定した場合、
    // ("containers[0].ports[1].host_port")と同じ形式でインデックス付きのfieldを返す。
    #[tokio::test]
    async fn update_rejects_nonexistent_container_name_with_indexed_field() {
        let state = test_state().await;
        registration::create(&state, compose_request())
            .await
            .unwrap();

        let result = update(
            &state,
            "webstack",
            UpdateServiceRequest {
                containers: Some(vec![ContainerInput {
                    name: "doesnotexist".to_string(),
                    ports: vec![],
                    volumes: vec![],
                }]),
                ..Default::default()
            },
        )
        .await;

        match result {
            Err(AppError::Validation(fields)) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].field, "containers[0].name");
            }
            other => panic!("VALIDATION_ERRORかつcontainers[0].nameを期待: {other:?}"),
        }
    }
}
