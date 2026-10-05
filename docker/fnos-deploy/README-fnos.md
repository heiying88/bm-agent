# 飞牛 OS（fnOS）Docker 部署 Bamboo

> 本目录两个包由 Windows 侧打包（2026-10-05）：
> - `bamboo-src.tar.gz`（9.4MB）：完整源码，**前端已嵌入 crate**
>   （`crates/app/bamboo-server/frontend_package/lotus-frontend.zip`，
>   含微信配置卡片 + 扫码登录 UI），NAS 上不需要 Lotus-main / node。
> - `bamboo-data.tar.gz`（99KB）：Windows 侧 `~/.bamboo` 的配置迁移包
>   （模型 provider 配置、加密密钥、凭据库、会话历史；**不含** frontend/
>   ——容器首次启动会自己解包；**微信平台配置为空**——迁移后在网页里
>   用"扫码登录"首配，正好走新流程）。

## 0. 传输与准备

1. 把 `bamboo-src.tar.gz`、`bamboo-data.tar.gz` 两个文件复制到 NAS
   （SMB 共享目录或 `scp` 均可），假设放在 `/vol1/docker/`。
2. SSH 登录飞牛 OS（fnOS 设置里开启 SSH）。

## 1. 解压源码并启动构建

```bash
mkdir -p /vol1/docker/bamboo && tar -xzf /vol1/docker/bamboo-src.tar.gz -C /vol1/docker/bamboo
cd /vol1/docker/bamboo/docker

# 家庭内网使用：把发布从"仅本机回环"改为局域网可访问
# （默认 127.0.0.1:9562:9562 在 NAS 上意味着 PC 浏览器打不开）
sed -i 's/127.0.0.1:9562:9562/9562:9562/' docker-compose.yml

# 时区（可选，日志时间戳对齐本地）
sed -i 's/- RUST_LOG=info/- RUST_LOG=info\n      - TZ=Asia\/Shanghai/' docker-compose.yml

# 构建并启动。首次构建要编译全部依赖（NAS CPU 上约 30–90 分钟），
# 之后 cargo-chef 缓存依赖层，改代码重建只需几分钟。
docker compose -p bamboo up -d --build
```

构建内存提示：release + LTO 峰值需要 ~4GB 内存。NAS 内存紧张时在
`docker-compose.yml` 的 `build:` 块加 `args:` 传 `CARGO_BUILD_JOBS=2`
（Dockerfile 需加一行 `ARG CARGO_BUILD_JOBS` + `ENV CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS`），
或先给 fnOS 加 swap。

## 2. 迁移 Windows 侧配置

```bash
docker compose -p bamboo stop

# 数据解进 named volume（项目名 bamboo → volume 名 bamboo_bamboo-data）
docker run --rm \
  -v bamboo_bamboo-data:/data \
  -v /vol1/docker:/mnt \
  alpine sh -c "tar -xzf /mnt/bamboo-data.tar.gz -C /data && chown -R 10001:10001 /data"

docker compose -p bamboo up -d
curl http://127.0.0.1:9562/api/v1/health   # → 应返回健康 JSON
```

迁移包带过去的：模型 provider（glm 等 API 配置）、加密密钥 + 凭据库
（成对迁移，互相能解开）、会话历史。**微信桥接是空的**——下一步首配。

## 3. 微信扫码首配（新功能首秀）

PC 浏览器打开 `http://<NAS_IP>:9562`：

1. 设置 → 系统设置 → Connect → 微信卡片
2. 打开微信启用开关
3. 允许的用户 ID：`o9cq8099v8BpzJwAHZ0jvF3EErnc@im.wechat`
4. 硅基流动 API Key：填你的 `sk-` 密钥
5. 语音开关：自动语音 `off`、投递 `file`、音频文件转写 `on`（当前偏好）
6. 点"开始扫码登录"→ 手机微信确认 → "✅ Token 已保存"

```bash
docker compose -p bamboo restart   # 重启让微信桥接加载新 token
```

## 4. 验证

```bash
docker logs bamboo 2>&1 | grep -a "wechat voice enabled"
# 期望：reply_mode=Off delivery=File file_asr=true ...
```

微信发条消息 → 文字回复；发个 mp3 → 模型直接按内容回答；
说"念一下" → mp3 语音文件回复。智能度/模型切换（/think、/model、
自然语言标记）只在微信入口生效。

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
