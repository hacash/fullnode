//! `mint-core` —— Hacash 共识动作定义与其执行所依赖的共识状态类型。
//!
//! 从 `mint`（矿工 crate）拆分而来，供三种消费者使用：
//! - `mint`（矿工）    依赖本 crate 复用铭刻动作与 MintState/MintTotal 状态类型；
//! - `sdk`（钱包 WASM）依赖本 crate 直接注册/构建/审阅铭刻动作（32-36），
//!   不再需要 codec 镜像；本 crate 无 tokio/x16rs，可编译到 wasm32；
//! - `app`（全节点）   经 mint/sdk 间接获得同一份定义。
//!
//! 存储布局兼容性：`MintTotal` 字段顺序、`TOTAL_KEY = b"_mint.total"` 与
//! channel 存储 key 均与拆分前一致（纯搬迁，无序列化变化），主网状态不受影响。

pub mod inscription;
pub mod setup;
pub mod state;
