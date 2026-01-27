//! NetMic 服务端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 通过日志明确后续需要接入的模块位置。

use netmic_proto::protocol::SessionParams;
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-server skeleton started");
    println!("netmic-server skeleton ready");
}

