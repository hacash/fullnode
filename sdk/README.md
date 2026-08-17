# Hacash Unified SDK 2.0

Unified SDK 2.0（doc 14 `unified-sdk-major-version-design.md`）重构后的
fullnode WASM SDK。设计要点：

- **无 v1/v2 命名空间**：一个表面、一个发布线；能力由
  `system.capabilities()` 的 feature/schema/profile 表达。
- **私钥不穿越 SDK 边界**：SDK 只产生 `SigningRequest`（digest + bindings）
  并消费 `SignatureProof`；钱包 vault 负责签名。
- **JSON transport 极小面**：raw WASM 只有 `sdk_invoke(request_json) ->
  envelope_json` 与 `sdk_transport_version()`，操作增加不改变 WASM 表面。
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
sdk.tx.inspect(body, signerAddress, context);        // 严格模式（高度/链 guard）
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
（只校验 body/signer/signature）。`tx.encode` 强制重建 body 的
`unsigned_body_hash` 与声明值一致，篡改 action json 会以
`transaction_json_mismatch` 失败；提供 `review` 时同样校验其 binding。

输入对象全部启用未知字段拒绝（拼错字段报 `unknown_field`/`unknown_action`
而不是静默忽略）；`system.capabilities().features` 与 dispatcher 共享同一
`OPERATIONS` 注册表（有测试保证两者不漂移）。审阅中的 `chain_ids_allowed`
为多个 `ChainAllow` 的交集（协议逐条执行），`valid_height_range` 同理取
交集。

错误统一为 `{ code, message, detail? }`（`SdkError`），facade 抛出带
`code`/`detail` 的异常，raw envelope 在 `e.sdkError` 中保留。

## 构建

前置：`wasm32-unknown-unknown` target、`wasm-bindgen-cli 0.2.100`、
`wasm-opt`（可选）。`wasm-opt` 需显式 `--enable-bulk-memory
--enable-sign-ext`（新版 rustc 的 wasm 输出包含这些指令，旧版
`--all-features` 会产出无效模块；build.sh 已处理并带校验回退）。

```sh
./sdk/pack.sh
```

产物在 `sdk/dist/`：`nodejs/`、`web/`、`page/`（base64 内联）、`js/`（facade）。

## Rust 原生使用

rlib 同样可用：`sdk::inspect::inspect_report`、`sdk::attach::*`、
`build_transaction`、`evaluate_policy` 等强类型 API（与 WASM 共用同一套
协议逻辑和向量）。

## 测试

```sh
cargo test -p sdk          # 41 个单元/流程测试（含黄金向量、签名流、guard 检查、篡改/过期/deny 拒绝）
node ./sdk/tests/...       # 打包后 JS 冒烟
```

## 状态（M1）

- 已实现：dispatcher、codec_profile/capabilities、decode/inspect/Review、
  prepare/attach/verify/signature_report、tx.build、tx.decode/encode、
  account/amount/message/policy、VM 40/41/44/46 注册、wasm 三端构建。
- 待办（M2+）：IDL/代码生成收口、AST/TEX 分支展示细化、VM maincall 的
  bytecode print/IR、wasm 体积优化（当前 gzip ~1.36MB，超出 §9 的 page
  1MB 预算，属 S0 体积 spike 事项）。
