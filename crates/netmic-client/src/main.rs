//! NetMic 客户端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 先使用共享默认参数，后续再接入采集/发送链路。

use netmic_proto::protocol::SessionParams;
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-client skeleton started");
    println!("netmic-client skeleton ready");
}

