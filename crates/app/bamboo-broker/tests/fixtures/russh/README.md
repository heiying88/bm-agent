# 真实 russh 传输夹具

仓库级 runner 会启动这个夹具并执行带忽略标记的 `russh_live` 集成测试：

```sh
scripts/run-russh-live.sh
```

该 runner 需要可用的 Docker 守护进程、`ssh-keygen` 和 Cargo。它会创建一个临时 Ed25519 客户端密钥，构建按摘要钉住的 Alpine 夹具，把 SSH 端口绑定到随机回环端口，等待容器健康检查通过，并在退出时总是清理容器、镜像标签和密钥目录。

容器在启动时生成全新的 Ed25519 主机密钥。只启用了非特权 `deploy` 用户的公钥认证；密码、root、agent、X11 以及本地转发均已禁用。远程转发和 `internal-sftp` 保持启用，因为它们正是被测试的生产契约。不使用任何仓库密钥或外部 SSH 服务。

对于已经在运行的 SSH 服务器，可直接用 `RUSSH_KEY_PATH`（推荐）或 `RUSSH_PASS` 调用这个被忽略的测试：

```sh
RUSSH_HOST=127.0.0.1 \
RUSSH_PORT=2222 \
RUSSH_USER=deploy \
RUSSH_KEY_PATH=/path/to/test_ed25519 \
cargo test --locked -p bamboo-broker --test russh_live \
  russh_deploys_through_reverse_tunnel -- --exact --ignored --nocapture
```

Rust 测试有 60 秒的契约超时。受保护的 Linux `Test` CI 作业还把夹具的完整构建与执行限制在十分钟内，这样启动或清理方面的回归会显式失败，而不是被静默跳过。
