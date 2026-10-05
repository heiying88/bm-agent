# 飞牛 OS（fnOS）Docker 部署 Bamboo

> 本目录内容（2026-10-05 由 Windows 侧打包）：
>
> | 文件 | 说明 |
> |---|---|
> | `bamboo-server-image.tar`（187MB） | **成品 Docker 镜像** `bamboo-server:latest`（amd64，未压缩 docker-save 规范布局，飞牛"导入镜像"界面可用；`docker load` 亦可）。内含 Debian bookworm-slim + 交叉编译二进制（OpenSSL 静态链入、前端已嵌入）+ CA 证书 + 运行用户 |
> | `bamboo-data.tar.gz`（99KB） | Windows 侧 `~/.bamboo` 迁移包（模型 provider 配置、加密密钥、凭据库、会话；**不含** frontend/，**微信配置为空**——到网页里扫码首配） |
> | `docker-compose.prebuilt.yml` | 启动编排（内网发布、安全加固、时区） |
> | `bamboo-linux.gz` + `Dockerfile.prebuilt` | 备选：NAS 上 `docker build` 现场构建（1 分钟，标准官方流程，兜底一切格式问题） |
> | `build-image.sh` | 无 Docker 引擎下重打镜像的脚本（改代码重新交叉编译后运行出新 tar） |
> | `bamboo-src.tar.gz`（9.4MB） | 完整源码（备选：从源码构建；Dockerfile 已默认 LTO off + jobs=2 适配小内存） |

## 路线 A（推荐）：导入成品镜像

### 0. 传输

把 `bamboo-server-image.tar`、`docker-compose.prebuilt.yml`、
`bamboo-data.tar.gz` 复制到 NAS（SMB 或 scp），假设放 `/vol1/docker/bamboo/`。

### 1. 导入镜像（秒级）

- 界面：fnOS → Docker → 镜像 → 导入 → 选 `bamboo-server-image.tar`
- 或 SSH：`docker load -i /vol1/docker/bamboo/bamboo-server-image.tar`

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

## 路线 C：fnOS"创建项目"（git 仓库方式，官方 compose 流程）

仓库根目录已放好 `docker-compose.yml`（fnOS 专用：构建源码 + 局域网发布 + 时区）。

1. 把仓库弄到 NAS 上（任选）：
   - NAS 上 `git clone https://github.com/heiying88/bm-agent.git /vol1/docker/bamboo`
   - 或把 Windows 工作区整个目录 SMB 拷到 `/vol1/docker/bamboo`（不含 target/）
2. 飞牛界面：Docker → 项目 → 创建项目 → 项目名 `bamboo`、路径
   `/vol1/docker/bamboo` → 启用 docker-compose.yml → 启动
   （首次构建 30–90 分钟；日志里能看到 cargo 编译进度）
3. 数据迁移与微信扫码首配同路线 A 第 2/3 步（volume 名为 `bamboo_bamboo-data`）

## 安全注意（来自 DEPLOY.md，必须知道）

**⚠️ 曾有密钥泄露事故（2026-10-05）**：`bamboo-data.tar.gz`（含数据目录
加密密钥）曾被误提交并推送到 GitHub，已从 git 移除并要求历史抹除 +
force push；若仓库曾为 public，请视为密钥已泄露——在网页设置里重录所有
API Key（重录会以（可能已泄露的）旧密钥加密？不会：凭据明文只在录入
瞬间出现，但**解密它们的对称密钥已泄露**，所以请先删除
`~/.bamboo/.bamboo_encryption_key` 与 `credentials.json` 让 bamboo 重新
生成，再重录所有密钥）。

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
