# 已知安全问题

本文档跟踪因上游依赖限制而无法立即解决的安全公告。

## 审计状态

最近审计：2026-04-24
命令：`cargo audit`

---

## 已修复

### RUSTSEC-2026-0104 — rustls-webpki (HIGH)
- **问题**：证书吊销列表解析中的可达 panic
- **原版本**：0.103.12
- **修复版本**：0.103.13
- **状态**：已通过 `cargo update -p rustls-webpki` 解决
- **提交**：见 git 历史中的 rustls-webpki 升级

---

## 等待上游修复

以下警告**仅供参考**（并非可利用的漏洞）。它们需要上游 crate 更新，不进行破坏性 API 变更就无法解决。

### RUSTSEC-2024-0384 — instant（不再维护）
- **版本**：0.1.13
- **依赖链**：`parking_lot 0.11.2` -> `wasm-timer 0.2.5` -> `reqwest-retry 0.7.0`
- **受阻于**：升级 `reqwest-retry` 需要 `reqwest 0.13` + `reqwest-middleware 0.5`
- **影响**：低。`instant` 是 WASM 兼容垫片；生产环境未使用（仅桌面/服务器目标）。
- **跟踪**：待 `reqwest-middleware` 0.5 迁移可行时，将 `reqwest-retry` 升级到 0.9+。

### RUSTSEC-2024-0436 — paste（不再维护）
- **版本**：1.0.15
- **依赖链**：`ratatui 0.29.0` -> `bamboo-tui`
- **受阻于**：`ratatui` 0.30 有破坏性 API 变更（`Widget` trait 移至 `ratatui-core`）
- **影响**：低。`paste` 是编译期宏 crate；无运行时暴露。
- **跟踪**：将 `ratatui` 升级到 0.30+ 并修复 `bamboo-tui` 的 widget 导入。

### RUSTSEC-2025-0134 — rustls-pemfile（不再维护）
- **版本**：2.2.0
- **依赖链**：`rustls-native-certs 0.7.3` -> `hyper-http-proxy 1.1.0` -> `launchdarkly-sdk-transport 0.1.1`
- **受阻于**：`rustls-native-certs` 0.8+ 可能有 API 变更
- **影响**：低。仅用于 GrowthBook/launchdarkly SDK 传输。
- **跟踪**：关注 `hyper-http-proxy` 和 `launchdarkly-sdk-transport` 的更新。

### RUSTSEC-2026-0002 — lru（不健全）
- **版本**：0.12.5
- **依赖链**：`ratatui 0.29.0` -> `bamboo-tui`
- **受阻于**：与 `paste` 相同——需要 `ratatui` 0.30+
- **影响**：低。`IterMut` 违规只影响 unsafe 代码路径；ratatui 的用法是安全的。
- **跟踪**：将 `ratatui` 升级到 0.30+（自带修复此问题的 `lru` 0.16+）。

---

## 解决计划

| 公告 | 工作量 | 预计时间 | 负责人 |
|----------|--------|-----|-------|
| RUSTSEC-2024-0384 (instant) | 中 | 下一次 reqwest 升级周期 | 后端 |
| RUSTSEC-2024-0436 (paste) | 低 | 随 ratatui 0.30 升级 | TUI |
| RUSTSEC-2025-0134 (rustls-pemfile) | 低 | 关注上游 | 后端 |
| RUSTSEC-2026-0002 (lru) | 低 | 随 ratatui 0.30 升级 | TUI |

---

## CI 配置

这些警告通过 `cargo audit` 默认配置在 CI 中被允许。它们将每月重新评估一次，或在重大依赖更新时重新评估。
