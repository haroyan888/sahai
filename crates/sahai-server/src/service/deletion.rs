//! 削除フロー: 「外側(Traefikルート)→中間(コンテナ)→中心(DBレコード)」の順に、
//! 外部からの流入経路を先に断ってから内側を消す。途中で失敗したらDBレコードは残す。

use crate::error::AppError;
use crate::repo::{services, ImmediateTransaction};
use crate::state::AppState;

pub async fn delete(
    state: &AppState,
    id_or_name: &str,
    purge_volumes: bool,
) -> Result<(), AppError> {
    let detail = super::load_detail(state, id_or_name).await?;

    // 1. 外側: Traefikルート削除
    state
        .traefik
        .remove_route(&detail.service.subdomain)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // 2. 中間: コンテナ/composeプロジェクト停止
    let runtime = state.docker.runtime_for(detail.service.source_type);
    runtime
        .stop(&detail)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // 3. 中心: DBレコード削除(CASCADEでcontainers/ports/volumesも削除)
    let mut tx = ImmediateTransaction::begin(&state.db).await?;
    match services::delete(tx.conn(), detail.service.id).await {
        Ok(()) => tx.commit().await?,
        Err(e) => {
            // registration.rs/update.rsと同じ理由でrollbackを明示する
            let _ = tx.rollback().await;
            return Err(e.into());
        }
    }

    if purge_volumes {
        let dir = state
            .config
            .services_volume_root()
            .join(detail.service.id.to_string());
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("ボリュームディレクトリの削除に失敗しました: {e}");
            }
        }
    }

    Ok(())
}
