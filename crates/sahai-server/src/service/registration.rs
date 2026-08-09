//! 登録: バリデーション→DB挿入(トランザクション)。
//! Traefikルートはここでは生成しない。起動していないサービスは個別ルートを持たず、
//! catch-allルート経由でNot Serviceページが応答するため、前もって書き出す必要がない。

use sahai_core::validation;

use crate::api::dto::CreateServiceRequest;
use crate::domain::{Protocol, ServiceDetail, SourceType};
use crate::error::{AppError, FieldError};
use crate::repo::{containers, ports, services, volumes, ImmediateTransaction};
use crate::state::AppState;

pub async fn create(
    state: &AppState,
    req: CreateServiceRequest,
) -> Result<ServiceDetail, AppError> {
    validate(&req)?;

    let source_type = SourceType::try_from(req.source_type.as_str())
        .map_err(|e| AppError::validation_single("source_type", e))?;

    match source_type {
        SourceType::Image if req.compose_content.is_some() => {
            return Err(AppError::Unprocessable(
                "source_type=imageの場合、compose_contentは指定できません".to_string(),
            ))
        }
        SourceType::Compose if req.image.is_some() => {
            return Err(AppError::Unprocessable(
                "source_type=composeの場合、imageは指定できません".to_string(),
            ))
        }
        _ => {}
    }

    let env_vars = req
        .env_vars
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));

    let domain = state.settings.read().await.domain.clone();
    let mut tx = ImmediateTransaction::begin(&state.db).await?;

    // 既存サービスとのhost_port衝突はDBを引かないと分からないため、
    // 直列化されたトランザクションの中で検証してから挿入する
    if let Err(e) =
        super::port_check::check_against_existing(tx.conn(), &req.containers, None).await
    {
        tx.rollback().await?;
        return Err(e);
    }

    let result = insert_all(tx.conn(), &req, source_type, &env_vars, &domain).await;

    let service_id = match result {
        Ok(id) => {
            tx.commit().await?;
            id
        }
        Err(e) => {
            // BEGIN IMMEDIATEしたコネクションをcommit/rollbackせずに手放すと、
            // プールに「トランザクション開始済み」のまま返却され、以降の全操作が
            // 壊れる(service::registration::tests::
            // failed_registration_does_not_poison_the_connection_poolで検証済み)
            let _ = tx.rollback().await;
            return Err(e);
        }
    };

    super::load_detail_by_id(state, service_id).await
}

async fn insert_all(
    conn: &mut sqlx::SqliteConnection,
    req: &CreateServiceRequest,
    source_type: SourceType,
    env_vars: &serde_json::Value,
    domain: &str,
) -> Result<i64, AppError> {
    let subdomain = sahai_core::naming::subdomain_for(&req.name, domain);
    let service_id = services::insert(
        &mut *conn,
        services::NewService {
            name: &req.name,
            subdomain: &subdomain,
            source_type,
            image: req.image.as_deref(),
            compose_content: req.compose_content.as_deref(),
            env_vars,
        },
    )
    .await?;

    for (i, container_input) in req.containers.iter().enumerate() {
        let container_id =
            containers::insert(&mut *conn, service_id, &container_input.name).await?;

        for (j, port_input) in container_input.ports.iter().enumerate() {
            let protocol = Protocol::try_from(port_input.protocol.as_str()).map_err(|e| {
                AppError::validation_single(format!("containers[{i}].ports[{j}].protocol"), e)
            })?;
            ports::insert(
                &mut *conn,
                container_id,
                &ports::NewPort {
                    container_port: port_input.container_port,
                    host_port: port_input.host_port,
                    protocol,
                    is_http: port_input.is_http,
                },
            )
            .await?;
        }

        for volume_input in &container_input.volumes {
            volumes::insert(&mut *conn, container_id, &volume_input.container_path).await?;
        }
    }

    // compose型の場合、compose_contentに含まれるがcontainers[]に明示されていない
    // サービス名も、ports/volumes空のServiceContainerとして自動作成する
    // (compose_content編集で新規追加されたサービスと同じ扱い。
    // validate()で既にcompose_contentのパース成功・containers[].nameの部分集合関係を
    // 検証済みのため、ここでのパース失敗は通常起こらない)
    if source_type == SourceType::Compose {
        if let Some(content) = &req.compose_content {
            let all_names = sahai_core::compose::parse_service_names(content)
                .map_err(|e| AppError::validation_single("compose_content", e.to_string()))?;
            let explicit_names: std::collections::HashSet<&str> =
                req.containers.iter().map(|c| c.name.as_str()).collect();
            for name in &all_names {
                if !explicit_names.contains(name.as_str()) {
                    containers::insert(&mut *conn, service_id, name).await?;
                }
            }
        }
    }

    Ok(service_id)
}

fn validate(req: &CreateServiceRequest) -> Result<(), AppError> {
    let mut errors = Vec::new();

    if let Err(e) = validation::validate_service_name(&req.name) {
        errors.push(FieldError {
            field: "name".to_string(),
            message: e.to_string(),
        });
    }

    let is_compose = req.source_type == "compose";
    let mut compose_service_names: Vec<String> = Vec::new();
    // compose_contentのパース自体に失敗した場合、compose_service_namesは空集合のままになり、
    // 後続のcontainers[].name整合性チェックが「全コンテナ名が見つからない」という
    // 無意味な二重エラーを追加してしまう。それを避けるためパース成否を別途記録する
    let mut compose_parsed_ok = !is_compose;
    if is_compose {
        if let Some(content) = &req.compose_content {
            match sahai_core::compose::parse_service_names(content) {
                Ok(names) => {
                    compose_service_names = names;
                    compose_parsed_ok = true;
                }
                Err(e) => errors.push(FieldError {
                    field: "compose_content".to_string(),
                    message: e.to_string(),
                }),
            }
            for name in &compose_service_names {
                if let Err(e) = validation::validate_compose_service_name(name) {
                    errors.push(FieldError {
                        field: format!("compose_content[{name}]"),
                        message: e.to_string(),
                    });
                }
                let tag = sahai_core::naming::registry_tag_name(&req.name, Some(name));
                if let Err(e) = validation::validate_registry_tag_length(&tag) {
                    errors.push(FieldError {
                        field: format!("compose_content[{name}]"),
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    // containers[].name の整合性チェック。
    // compose_content自体のパースに失敗している場合は、二次的な無意味なエラーを
    // 追加しないためスキップする
    if is_compose && compose_parsed_ok {
        for (i, c) in req.containers.iter().enumerate() {
            if !compose_service_names.contains(&c.name) {
                errors.push(FieldError {
                    field: format!("containers[{i}].name"),
                    message: format!("'{}' はcompose_contentに存在しないサービス名です", c.name),
                });
            }
        }
    } else if !is_compose {
        if req.containers.len() != 1 {
            errors.push(FieldError {
                field: "containers".to_string(),
                message: "image型はcontainersを1件だけ指定してください".to_string(),
            });
        } else if req.containers[0].name != req.name {
            errors.push(FieldError {
                field: "containers[0].name".to_string(),
                message: "image型ではcontainers[0].nameはnameと完全一致させてください".to_string(),
            });
        }
    }

    errors.extend(super::port_check::collect_request_errors(&req.containers));

    let mut http_count = 0;
    for c in &req.containers {
        for p in &c.ports {
            if p.is_http {
                http_count += 1;
            }
        }
    }
    if http_count > 1 {
        errors.push(FieldError {
            field: "containers[].ports[].is_http".to_string(),
            message: "is_httpはサービスにつき最大1件までです".to_string(),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(errors))
    }
}

#[cfg(test)]
mod validate_tests {
    use super::validate;
    use crate::api::dto::{ContainerInput, CreateServiceRequest, PortInput};
    use crate::error::AppError;

    fn fields_of(result: &Result<(), AppError>) -> Vec<&str> {
        match result {
            Err(AppError::Validation(fields)) => fields.iter().map(|f| f.field.as_str()).collect(),
            _ => vec![],
        }
    }

    fn image_ok() -> CreateServiceRequest {
        CreateServiceRequest {
            name: "myapp".to_string(),
            source_type: "image".to_string(),
            image: Some("x:latest".to_string()),
            compose_content: None,
            env_vars: None,
            containers: vec![ContainerInput {
                name: "myapp".to_string(),
                ports: vec![PortInput {
                    container_port: 80,
                    host_port: Some(20001),
                    protocol: "tcp".to_string(),
                    is_http: true,
                }],
                volumes: vec![],
            }],
        }
    }

    fn compose_ok() -> CreateServiceRequest {
        CreateServiceRequest {
            name: "webstack".to_string(),
            source_type: "compose".to_string(),
            image: None,
            compose_content: Some(
                "services:\n  app:\n    build: .\n  mysql:\n    image: mysql:8\n".to_string(),
            ),
            env_vars: None,
            containers: vec![ContainerInput {
                name: "app".to_string(),
                ports: vec![PortInput {
                    container_port: 80,
                    // is_httpのポートはホストに公開しないためhost_portを持たない
                    host_port: None,
                    protocol: "tcp".to_string(),
                    is_http: true,
                }],
                volumes: vec![],
            }],
        }
    }

    /// host_portの検証は非HTTPポートにだけ効く。検証系のテストはこれを足して行う
    fn with_non_http_port(req: &mut CreateServiceRequest, host_port: i64) {
        req.containers[0].ports.push(PortInput {
            container_port: 3306,
            host_port: Some(host_port),
            protocol: "tcp".to_string(),
            is_http: false,
        });
    }

    #[test]
    fn accepts_valid_image_request() {
        assert!(validate(&image_ok()).is_ok());
    }

    #[test]
    fn accepts_valid_compose_request_with_partial_containers() {
        // mysqlはcontainersに未記載でも良い(6章: 空のports/volumesで作成される)
        assert!(validate(&compose_ok()).is_ok());
    }

    #[test]
    fn rejects_invalid_service_name() {
        let mut req = image_ok();
        req.name = "BadName".to_string();
        req.containers[0].name = "BadName".to_string();
        let result = validate(&req);
        assert_eq!(fields_of(&result), vec!["name"]);
    }

    #[test]
    fn rejects_reserved_service_names() {
        // 予約語は管理画面のサブドメインと衝突する名前。
        // リストが変わってもこのテストが追随するよう、定義そのものを走査する
        for &reserved in sahai_core::validation::RESERVED_SERVICE_NAMES {
            let mut req = image_ok();
            req.name = reserved.to_string();
            req.containers[0].name = reserved.to_string();
            let result = validate(&req);
            assert_eq!(
                fields_of(&result),
                vec!["name"],
                "'{reserved}'は拒否されるべき"
            );
        }
    }

    #[test]
    fn image_type_requires_exactly_one_container() {
        let mut req = image_ok();
        req.containers.push(ContainerInput {
            name: "extra".to_string(),
            ports: vec![],
            volumes: vec![],
        });
        assert_eq!(fields_of(&validate(&req)), vec!["containers"]);
    }

    #[test]
    fn image_type_container_name_must_match_service_name() {
        let mut req = image_ok();
        req.containers[0].name = "othername".to_string();
        assert_eq!(fields_of(&validate(&req)), vec!["containers[0].name"]);
    }

    #[test]
    fn compose_type_rejects_container_name_not_in_compose_content() {
        let mut req = compose_ok();
        req.containers[0].name = "notindockercompose".to_string();
        // インデックス付き(例: "containers[0].ports[1].host_port")で返し、
        // 同じ形式)。どのcontainers要素が悪いかWeb UI側で一意に特定できるようにする
        assert_eq!(fields_of(&validate(&req)), vec!["containers[0].name"]);
    }

    /// 範囲による制限は設けていない。特定の帯に限定する変更が入れば落ちる。
    #[test]
    fn accepts_host_port_from_any_band() {
        let mut req = image_ok();
        with_non_http_port(&mut req, 8080);
        assert!(validate(&req).is_ok());
    }

    /// is_httpのポートはホストに公開しないため、host_portの検証対象外。
    /// 予約ポートを指定しても無視される(そもそも公開されない)。
    #[test]
    fn ignores_host_port_on_http_ports() {
        let mut req = image_ok();
        req.containers[0].ports[0].host_port = Some(443);
        assert!(validate(&req).is_ok());
    }

    #[test]
    fn rejects_port_used_by_sahai_itself() {
        let mut req = image_ok();
        with_non_http_port(&mut req, 443);
        assert_eq!(
            fields_of(&validate(&req)),
            vec!["containers[0].ports[1].host_port"]
        );
    }

    #[test]
    fn rejects_duplicate_host_port_within_one_request() {
        let mut req = image_ok();
        with_non_http_port(&mut req, 20005);
        with_non_http_port(&mut req, 20005);
        assert_eq!(
            fields_of(&validate(&req)),
            vec!["containers[0].ports[2].host_port"]
        );
    }

    #[test]
    fn rejects_more_than_one_http_port() {
        let mut req = compose_ok();
        req.containers.push(ContainerInput {
            name: "mysql".to_string(),
            ports: vec![PortInput {
                container_port: 3306,
                host_port: Some(20002),
                protocol: "tcp".to_string(),
                is_http: true,
            }],
            volumes: vec![],
        });
        assert_eq!(
            fields_of(&validate(&req)),
            vec!["containers[].ports[].is_http"]
        );
    }

    #[test]
    fn rejects_invalid_compose_yaml() {
        let mut req = compose_ok();
        req.compose_content = Some("not: [valid".to_string());
        assert_eq!(fields_of(&validate(&req)), vec!["compose_content"]);
    }

    #[test]
    fn accumulates_multiple_errors_at_once() {
        // fail-fastにせず複数まとめて返す
        let mut req = image_ok();
        req.name = "BadName".to_string();
        with_non_http_port(&mut req, 80);
        let result = validate(&req);
        let fields = fields_of(&result);
        assert!(fields.contains(&"name"));
        assert!(fields.contains(&"containers[0].ports[1].host_port"));
    }

    #[test]
    fn field_paths_include_indices_for_multiple_containers_and_ports() {
        // 複数コンテナ・複数ポートがある場合、どの要素かを一意に特定できることを確認
        let mut req = compose_ok();
        req.containers.push(ContainerInput {
            name: "mysql".to_string(),
            ports: vec![
                PortInput {
                    container_port: 3306,
                    host_port: Some(20002), // 1件目のポートは問題なし
                    protocol: "tcp".to_string(),
                    is_http: false,
                },
                PortInput {
                    container_port: 3307,
                    host_port: Some(443), // 差配自身が使う予約ポート
                    protocol: "tcp".to_string(),
                    is_http: false,
                },
            ],
            volumes: vec![],
        });
        assert_eq!(
            fields_of(&validate(&req)),
            vec!["containers[1].ports[1].host_port"]
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::api::dto::{ContainerInput, CreateServiceRequest, PortInput};
    use crate::service::{registration::create, test_support::test_state};

    fn image_request(name: &str, host_port: Option<i64>) -> CreateServiceRequest {
        CreateServiceRequest {
            name: name.to_string(),
            source_type: "image".to_string(),
            image: Some("x:latest".to_string()),
            compose_content: None,
            env_vars: None,
            containers: vec![ContainerInput {
                name: name.to_string(),
                ports: vec![PortInput {
                    container_port: 80,
                    host_port,
                    protocol: "tcp".to_string(),
                    is_http: true,
                }],
                volumes: vec![],
            }],
        }
    }

    // RED: host_portのUNIQUE制約違反はservices::insert/containers::insert成功後、
    // ports::insertの段階(=トランザクション開始後)で起きる。この時ImmediateTransactionが
    // commit/rollbackされずに破棄されると、コネクションプール(max_connections=1)に
    // 「BEGIN IMMEDIATEしたまま」のコネクションが返却され、後続の操作全てが
    // 壊れることを検証する。
    #[tokio::test]
    async fn failed_registration_does_not_poison_the_connection_pool() {
        let state = test_state().await;

        create(&state, image_request("first", Some(21001)))
            .await
            .unwrap();

        let conflict = create(&state, image_request("second", Some(21001))).await;
        assert!(
            conflict.is_err(),
            "host_port重複は失敗するはず: {conflict:?}"
        );

        // ここでプールのコネクションが汚染されていなければ、以降の正常な登録は
        // 問題なく完了するはず(acquire_timeout(3秒)で明確に失敗する設定済み)
        let third = create(&state, image_request("third", Some(21002))).await;
        assert!(
            third.is_ok(),
            "直前の失敗がコネクションを汚染していないはず: {third:?}"
        );
    }

    // RED: compose型の登録時、containers[]に明示されていないcompose_content内の
    // サービス名も、ports/volumes空のServiceContainerとして自動的に作成されるべき
    // (compose_content編集で新規追加されたサービスと同じ扱い。
    // これまでinsert_all()がreq.containers
    // だけをループしていたため実装されていなかった)。
    #[tokio::test]
    async fn compose_registration_auto_creates_containers_not_explicitly_listed() {
        let state = test_state().await;

        let detail = create(
            &state,
            CreateServiceRequest {
                name: "webstack".to_string(),
                source_type: "compose".to_string(),
                image: None,
                compose_content: Some(
                    "services:\n  web:\n    image: nginx\n  db:\n    image: mysql:8\n".to_string(),
                ),
                env_vars: None,
                containers: vec![ContainerInput {
                    name: "web".to_string(),
                    ports: vec![PortInput {
                        container_port: 80,
                        host_port: Some(20030),
                        protocol: "tcp".to_string(),
                        is_http: true,
                    }],
                    volumes: vec![],
                }],
            },
        )
        .await
        .unwrap();

        let names: Vec<&str> = detail
            .containers
            .iter()
            .map(|c| c.container.name.as_str())
            .collect();
        assert!(names.contains(&"web"));
        assert!(
            names.contains(&"db"),
            "containers[]に書かれていないdbもcompose_contentから自動作成されるべき: {names:?}"
        );

        let web = detail
            .containers
            .iter()
            .find(|c| c.container.name == "web")
            .unwrap();
        assert_eq!(
            web.ports.len(),
            1,
            "明示的に指定したwebのportsは維持されるべき"
        );

        let db = detail
            .containers
            .iter()
            .find(|c| c.container.name == "db")
            .unwrap();
        assert!(db.ports.is_empty(), "自動作成されたdbはports空であるべき");
        assert!(
            db.volumes.is_empty(),
            "自動作成されたdbはvolumes空であるべき"
        );
    }
}
