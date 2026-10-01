# 操作指南：部署 `bamboo serve`

以长期运行的服务器方式跑 Bamboo 有三种途径，隔离程度依次递增：裸二进制、systemd unit、Docker。

## 方式 1 —— 裸二进制

```bash
cargo install --path .          # 或者：cargo install bamboo-agent
bamboo init --non-interactive --provider anthropic --api-key "sk-ant-..."
bamboo serve
```

适合单用户的桌面/笔记本环境，或者作为 sidecar 进程管理器（如 Bodhi）直接拉起的目标。

## 方式 2 —— systemd（裸机 Linux 服务器）

```ini
# /etc/systemd/system/bamboo.service
[Unit]
Description=Bamboo agent server
After=network.target

[Service]
Type=simple
User=bamboo
Environment=BAMBOO_DATA_DIR=/var/lib/bamboo
Environment=BAMBOO_BIND=127.0.0.1
Environment=BAMBOO_PORT=9562
ExecStart=/usr/local/bin/bamboo serve
Restart=on-failure
RestartSec=5
# 加固（可选，但对常驻服务推荐）
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/bamboo
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```bash
sudo useradd --system --home /var/lib/bamboo bamboo
sudo mkdir -p /var/lib/bamboo && sudo chown bamboo:bamboo /var/lib/bamboo
sudo -u bamboo BAMBOO_DATA_DIR=/var/lib/bamboo bamboo init --non-interactive --provider anthropic --api-key "sk-ant-..."
sudo systemctl enable --now bamboo
```

## 方式 3 —— Docker

```bash
cd docker && docker compose up -d --build
curl http://localhost:9562/api/v1/health
```

`docker-compose.yml`（位于 `docker/`）默认就已经把该做的事都做对了：

- **仅发布到主机回环地址**（`127.0.0.1:9562:9562`）——为什么这很重要，
  见下文[网络暴露](#网络暴露绝不能跳过的部分)一节。
- 以非 root 用户运行，丢弃全部 Linux capabilities（`cap_drop: [ALL]`，
  agent 用不到），设置 `no-new-privileges`，并限制 `pids_limit`。
- 使用隔离的命名卷（`bamboo-data`），而不是把你整个 `~/.bamboo` 以读写
  方式绑定挂载进容器。如果你确实想共享宿主机上的 profile，把 compose
  文件里备用的 bind-mount 行取消注释即可。
- 设置 `BAMBOO_DATA_DIR=/data`、`BAMBOO_PORT=9562`、`BAMBOO_BIND=0.0.0.0`
  （这是容器内绑定——对宿主机的实际暴露由上面 `ports:` 发布层控制，而不是
  这个绑定地址）。

配置 provider 有两种方式：首次启动前把 `config.json` 挂载进卷，或者把
[provider API key 环境变量](../config-reference.md#environment-variables)
（`BAMBOO_ANTHROPIC_API_KEY` 等）加进 `environment:` 块——这些值只存在于内存，从不落盘，是容器/CI 部署的首选模式。如果你更愿意挂载文件，`docker/config.example.json` 可以作为起点。

## 网络暴露——绝不能跳过的部分

**在没加一层带认证的反向代理之前，不要把 Docker 发布（或任何绑定）扩大到 `0.0.0.0`/局域网 IP。** 有两个问题会叠加：

1. 全新实例没有配置任何凭据——在你设置密码之前，访问控制这道门是无效的。
2. 即使**已经**设置了密码，服务器也会按设计把所有私网（RFC1918）对端视为
   可信本地并跳过密码校验（桌面模式的便利性）——于是同一子网内的任何主机
   都能不经认证地触达能执行工具的 agent。

保持仅回环的发布/绑定，哪条网络需要远程访问，就在它前面放一个真正的反向代理（nginx、Caddy、Traefik），在那一层终结 TLS 并做自己的认证。或者，如果实在没法用独立代理，可设置 `server.tls`（`config.json` 中的 `cert_file`/`key_file`——参见[配置参考](../config-reference.md#server)）在 Bamboo 内部手动终结 TLS。

## 反向代理示例（Caddy）

```
bamboo.example.com {
  reverse_proxy 127.0.0.1:9562
  basicauth {
    admin JDJhJDE0JC4uLg==   # bcrypt 哈希，用 `caddy hash-password` 生成
  }
}
```

Caddy 在仅回环的 Bamboo 前面处理 TLS（通过 Let's Encrypt）和 HTTP basic auth——只要超出单个可信局域网的范围，就推荐用这个模式，而不是 Bamboo 自带的 `server.tls`/`access_control`。

## CORS

如果有基于浏览器的客户端（例如部署在其他 origin 的自托管 Lotus 前端）直接访问这台服务器，请把 `BAMBOO_CORS_ALLOW_ORIGINS`（或等价的配置键）设为显式允许列表——完整 origin（`https://app.example.com`）、裸主机名（`app.example.com`）或通配符子域名（`*.example.com`）。纯同源场景（例如 Bodhi 内嵌的 sidecar，它直接访问 `127.0.0.1`）留空即可。

## 备份部署

把整个数据目录（`BAMBOO_DATA_DIR`，默认 `~/.bamboo`）作为一个整体备份——它装着 `config.json`、`connect.json`、`schedules.json`、`model_limits.json`、全部会话，以及（至关重要的）`.bamboo_encryption_key`。在没有 `BAMBOO_CONFIG_ENCRYPTION_KEY` 副本的情况下丢失密钥文件，会让该目录中所有静态加密的机密（provider API key、IM 桥接 token、通知推送 token……）永久无法恢复——参见[静态加密](../config-reference.md#encryption-at-rest)。

## 健康检查

`GET /api/v1/health`（上面 Docker 示例使用的）或 `bamboo health`（同一项检查，从 CLI 执行）——服务器不可达或不健康时两者都会以非零值退出/返回，因此任何一个都能当就绪/存活探针用。
