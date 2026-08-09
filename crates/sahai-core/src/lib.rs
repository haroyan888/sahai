//! sahai-core: DB・Docker・HTTPに依存しない純粋ロジック。
//! sahai-server(Control Plane)とsahai-cliの両方から利用される。
//! I/O(DB・Docker・HTTP)を持たない純粋ロジックのみを置く。

pub mod compose;
pub mod docker_args;
pub mod error;
pub mod naming;
pub mod validation;

pub use error::CoreError;
