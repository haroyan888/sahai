#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("{0}")]
    Validation(String),

    #[error("compose_contentの解析に失敗しました: {0}")]
    ComposeParse(String),
}
