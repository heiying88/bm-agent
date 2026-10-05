# 飞牛 OS（fnOS）Docker 部署 Bamboo

> 本目录内容（2026-10-05 由 Windows 侧打包）：
>
> | 文件 | 说明 |
> |---|---|
> | `bamboo-linux.gz`（50MB） | **已交叉编译好的 Linux x86-64 二进制**（glibc 2.36 / bookworm，OpenSSL 已静态链入，前端已嵌入）。免构建路线用这个 |
> | `Dockerfile.prebuilt` + `docker-compose.prebuilt.yml` | 免构建镜像定义（NAS 上秒级构建） |
> | `bamboo-data.tar.gz`（99KB） | Windows 侧 `~/.bamboo` 迁移包（模型 provider 配置、加密密钥、凭据库、会话；**不含** frontend/，**微信配置为空**——到网页里扫码首配） |
> | `bamboo-src.tar.gz`（9.4MB） | 完整源码（备选：NAS 上从源码构建；Dockerfile 已默认 LTO off + jobs=2 适配小内存，NAS ≥4GB 可编） |

## 路线 A（推荐）：免构建，秒级起容器

### 0. 传输

把 `bamboo-linux.gz`、`Dockerfile.prebuilt`、`docker-compose.prebuilt.yml`、
`bamboo-data.tar.gz` 四个文件复制到 NAS（SMB 或 scp），假设放 `/vol1/docker/bamboo/`。
SSH 登录飞牛 OS。

### 1. 解压二进制并构建镜像（约 1 分钟，无编译）

```bash
cd /vol1/docker/bamboo
gunzip bamboo-linux.gz && chmod +x bamboo-linux
docker build -f Dockerfile.prebuilt -t bamboo-server:latest .
```

### 2. 启动并迁移数据

```bash
docker compose -f docker-compose.prebuilt.yml -p bamboo up -d
docker compose -f docker-compose.prebuilt.yml -p bamboo stop

# 数据灌进 named volume（项目名 bamboo → volume 名 bamboo_bamboo-data）
docker run --rm \
  -v bamboo_bamboo-data:/data \
  -v /vol1/docker/bamboo:/mnt \
  alpine sh -c "tar -xzf /mnt/bamboo-data.tar.gz -C /data && chown -R 10001:10001 /data"

docker compose -f docker-compose.prebuilt.yml -p bamboo up -d
curl http://127.0.0.1:9562/api/v1/health   # → 健康 JSON
```

### 3. 微信扫码首配

PC 浏览器打开 `http://<NAS_IP>:9562` → 设置 → 系统设置 → Connect → 微信卡片：

1. 打开微信启用开关
2. 允许的用户 ID：`o9cq8099v8BpzJwAHZ0jvF3EErnc@im.wechat`
3. 硅基流动 API Key：填你的 `sk-` 密钥
4. 语音开关：自动语音 `off`、投递 `file`、音频文件转写 `on`
5. 点"开始扫码登录"→ 手机微信确认 → "✅ Token 已保存"

```bash
docker compose -f docker-compose.prebuilt.yml -p bamboo restart
docker logs bamboo 2>&1 | grep -a "wechat voice enabled"
# 期望：reply_mode=Off delivery=File file_asr=true ...
```

## 路线 B（备选）：NAS 上从源码构建

```bash
mkdir -p /vol1/docker/bamboo-src && tar -xzf bamboo-src.tar.gz -C /vol1/docker/bamboo-src
cd /vol1/docker/bamboo-src/docker
sed -i 's/127.0.0.1:9562:9562/9562:9562/' docker-compose.yml   # 内网可访问
docker compose -p bamboo up -d --build     # 30–90 分钟（依赖层之后重建只要几分钟）
```

内存紧张（≤4GB）：Dockerfile 已默认 `BAMBOO_LTO=false`/`BAMBOO_JOBS=2`；
大内存机器可用 `--build-arg BAMBOO_LTO=true --build-arg BAMBOO_JOBS=8` 恢复。
数据迁移同路线 A 第 2 步。

## 安全注意（来自 DEPLOY.md，必须知道）

`9562:9562` 的局域网发布意味着：**同一内网的任何设备**都能无认证使用
这个 agent（服务端按设计把 RFC1918 内网对端视为可信本地、跳过密码）。
家庭内网可接受；若 NAS 暴露在不可信网络，改回 `127.0.0.1:9562:9562`
并前置带认证的反向代理（fnOS 应用中心有反向代理功能）。

## 日常维护

```bash
cd /vol1/docker/bamboo/docker
docker compose -p bamboo logs -f --tail=100   # 看日志
docker compose -p bamboo restart              # 重启
# 更新版本：替换源码后重建（依赖层缓存，只需几分钟）
docker compose -p bamboo up -d --build
# 数据备份：打包 volume
docker run --rm -v bamboo_bamboo-data:/data -v /vol1/docker:/mnt alpine \
  tar -czf /mnt/bamboo-data-backup.tar.gz -C /data .
```
