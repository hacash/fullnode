# Hacash Unified SDK 2.0

Unified SDK 2.0（doc 14 `unified-sdk-major-version-design.md`）重构后的
fullnode WASM SDK。设计要点：

- **无 v1/v2 命名空间**：一个表面、一个发布线；能力由
  `system.capabilities()` 的 feature/schema/profile 表达。
- **私钥不穿越 SDK 边界**：SDK 只产生 `SigningRequest`（digest + bindings）
  并消费 `SignatureProof`；钱包 vault 负责签名。
- **JSON-free WASM 核心 + 二进制极小面**：raw WASM 只有
  `sdk_invoke_binary(operation_id, payload)` 与 `sdk_transport_version()`；
  wasm 核心不解析/不产生任何 JSON（`bjson` 二进制字段流进、二进制 envelope
  出），全部 JSON 语义归 JS facade（原生 `JSON.parse/stringify`）。操作增加
  不改变 WASM 表面。
- **交易状态机**：`tx.build → inspect → prepare_signature → (vault)
  attach_signature → verify`；审阅对象 `Review` 本地生成，绑定
  `review_binding`。
- **kind registry 扩展**：新增 action 只增加 registry + schema，不改通用
  函数形状。

## 领域 API（JS facade）

```js
const sdk = await create_hacash_sdk({ target: "node" }); // auto|node|web

sdk.system.capabilities();      // abi / features / codec_profile_hash
sdk.system.codec_profile();     // fullnode_commit / limits / registered_kinds

sdk.tx.build({ spec });                              // 未签名 Type-2/3 body
sdk.tx.inspect_report(body, signerAddress?);         // 无链上下文审阅
sdk.tx.inspect(body, signerAddress, context);        // 严格模式（guard 作为事实报告，不拒绝）
sdk.tx.prepare_signature(body, signerAddress, opts); // SigningRequest
sdk.tx.attach_signature(body, proof, review, request);
sdk.tx.attach_signature_unbound(body, proof);        // 低级路径，无审批链
sdk.tx.verify(body);
sdk.tx.signature_report(body);
sdk.tx.decode(body) / sdk.tx.encode(transactionJson, review?); // 低级结构化 codec

sdk.account.verify_address(address);
sdk.account.address_from_public_key(publicKey);      // 无私钥输入
sdk.amount.parse_protocol(value) / format_protocol(value, unit); // 精确十进制字符串
sdk.message.prepare_signature(params) / verify(request, proof); // 冻结 raw 约定
sdk.policy.evaluate(review, policy);
```

`tx.prepare_signature` 的 `opts.policy` 由 SDK 自行评估：`deny` 决策直接拒绝
（`policy_denied`），`allow/confirm` 决策作为 `PolicyDecision` 绑定进
`SigningRequest`，调用方无法伪造决策结果。`tx.attach_signature`（完整链）
强制要求 `review` + `request`：request 的 id/binding 会被重算校验（篡改
任何字段——含 `expires_at`——都会以 `invalid_signing_request` 失败），并
校验 digest/body_hash/signer/purpose/algorithm、proof↔request 绑定、policy
决策与 review binding；无审批链的冷签名路径使用 `attach_signature_unbound`
（只校验 body/signature/limits；非必需签名者仅在 type-3 被拒绝——链的
精确 D 集规则，type-1/2 链容忍多余签名，SDK 同样放行并以
`complete`/`missing_signers` 报告完整性）。`tx.encode` 强制重建 body 的
`unsigned_body_hash` 与声明值一致，篡改 action json 会以
`transaction_json_mismatch` 失败；提供 `review` 时同样校验其 binding。

输入对象全部启用未知字段拒绝（拼错字段报 `unknown_field`/`unknown_action`
而不是静默忽略）；`system.capabilities().features` 与 dispatcher 共享同一
`OPERATIONS` 注册表（有测试保证两者不漂移）。审阅中的 `chain_ids_allowed`
为多个 `ChainAllow` 的交集（协议逐条执行），`valid_height_range` 同理取
交集；严格模式的 `expired_height`/`wrong_chain` 是调用者 context 下的派生
事实，SDK 从不因它们拒绝返回 review——是否继续由上层判断。

错误统一为 `{ code, message, detail? }`（`SdkError`），facade 抛出带
`code`/`detail` 的异常，raw envelope 在 `e.sdkError` 中保留。

## 构建

前置（一次性）：`rustup target add wasm32-unknown-unknown`、
`cargo install -f wasm-bindgen-cli --version 0.2.100`（版本已固定，
build.sh 会校验并给出安装提示）；`wasm-opt` 与 JS 压缩器（esbuild /
terser，或经 npx 按需下载的 esbuild）可选，缺失时自动降级。`wasm-opt`
需显式 `--enable-bulk-memory --enable-sign-ext`（新版 rustc 的 wasm
输出包含这些指令，旧版 `--all-features` 会产出无效模块；build.sh 已
处理并带校验回退）。

```sh
./sdk/pack.sh             # 常规构建（可读 JS）
./sdk/pack.sh --release   # 压缩 JS（facade/codec/glue/page 全部 minify）
```

一条命令完成全部：由 Rust schema 重新生成 TS/JS codec
（`codec-schema-gen` → `js/generated/`，构建产物不提交）、构建
nodejs/web/no-modules 三个 wasm 目标、装配 dist、可选压缩。产物全部在
`sdk/dist/`：

- `js/hacashsdk.mjs` — 友好 API 入口（node 自动加载 `../nodejs/`；
  web 用 `create_hacash_sdk({ wasm })` 加载 `../web/`）；
  `js/generated/` 为生成的 codec（`codec.mjs` + `codec.ts`）。
- `nodejs/`、`web/` — 对应平台的 wasm-bindgen 低级胶水 + wasm。
- `page/` — 浏览器 script 标签单文件：`hacashsdk_bg.js`（wasm base64
  内联）+ 演示页 `friendly_test.html`。

`dist/`、`js/generated/`、`*.bak` 等构建产物已被 git 忽略（见
`sdk/.gitignore`）；`sdk/check-schema.sh` 可独立验证已生成的 codec 与
Rust schema 一致。

### 关于 execute-off 构建的 unused import 警告

SDK 以 `default-features = false` 编译 protocol/vm/mint-core（`execute`
关闭），但它们与 fullnode 共享同一份源码。为保持 fullnode 侧代码整洁，
execute 相关的实现不再逐行用 cfg 裁剪，而是**全量编译、由 wasm 链接器
剥离死代码**；只有少数类型级切分（`ActionRef`/`TxRef` 等）和 mint-core
的 x16rs 硬依赖（C 代码无法为 wasm32 编译）仍保留 cfg。

因此 execute-off 构建会输出一批 `unused import` 警告——这些导入只被
execute 体使用，在 SDK 构建下确实是死代码。这是换取 fullnode 代码整洁
的刻意代价：**不要为了消除这些警告而恢复 per-import cfg**。它们不影响
fullnode 编译，也不影响 wasm 构建物（死代码剥离后产物不含 execute 代码，
`check-wasm-graph.sh` 负责守护这一点）。

## Rust 原生使用

rlib 同样可用：`sdk::inspect::inspect_report`、`sdk::attach::*`、
`build_transaction`、`evaluate_policy` 等强类型 API（与 WASM 共用同一套
协议逻辑和向量）。

## 测试

```sh
cargo test -p sdk          # 73 个单元/流程测试（含黄金向量、签名流、guard 事实、篡改/过期/deny 拒绝）
node ./sdk/tests/...       # 打包后 JS 冒烟
```

## 状态（M1）

- 已实现：dispatcher、codec_profile/capabilities、decode/inspect/Review、
  prepare/attach/verify/signature_report、tx.build、tx.decode/encode、
  account/amount/message/policy、VM 40/41/44/46 注册、wasm 三端构建。
- 待办（M2+）：IDL/代码生成收口、AST/TEX 分支展示细化、VM maincall 的
  bytecode print/IR、wasm 体积优化（当前 gzip ~1.36MB，超出 §9 的 page
  1MB 预算，属 S0 体积 spike 事项）。
