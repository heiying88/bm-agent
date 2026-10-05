#!/usr/bin/env bash
# 无 Docker 引擎环境下手工组装 docker-save 格式镜像（Windows Git Bash / Linux 通用）。
# 产物：bamboo-server-image.tar.gz —— `docker load -i` 或 fnOS"导入镜像"直接可用。
#
# 依赖：curl、tar（GNU）、node（仅 JSON 解析）、7z 或 7za（解 deb）、sha256sum。
# 网络：经 http_proxy 访问 Docker Hub + deb.debian.org（无代理时去掉 export）。
#
# 层结构（对齐 docker/Dockerfile.prebuilt 的运行时层）：
#   layer 1: debian:bookworm-slim（官方，amd64）
#   layer 2: 本脚本组装 —— /usr/local/bin/bamboo + CA 证书 + bamboo 用户 + /data
#   镜像 config: User 10001、ENV、ENTRYPOINT bamboo、CMD serve、
#   HEALTHCHECK 用 bash /dev/tcp（镜像内无 curl）。

set -euo pipefail

OUT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK="${TMPDIR:-/tmp}/bamboo-image-build"
BASE_IMAGE="debian"
BASE_TAG="bookworm-slim"
BINARY="${1:-$OUT_DIR/bamboo-linux}"
IMAGE_TAG="bamboo-server:latest"

export PATH="$PATH:/c/Program Files/nodejs:/c/Program Files/7-Zip"
export https_proxy="${https_proxy:-http://127.0.0.1:12450}"
export http_proxy="${http_proxy:-http://127.0.0.1:12450}"

command -v curl >/dev/null && command -v node >/dev/null && command -v sha256sum >/dev/null \
  || { echo "缺少 curl/node/sha256sum"; exit 1; }
[ -f "$BINARY" ] || { echo "找不到二进制：$BINARY（先放同目录或传参）"; exit 1; }

JSON_GET() { node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{const o=JSON.parse(d);console.log(eval('o'+process.argv[1]))})" "$1"; }

rm -rf "$WORK" && mkdir -p "$WORK/blobs" "$WORK/stage"
cd "$WORK"

echo "== 1/6 拉取 $BASE_IMAGE:$BASE_TAG (amd64) 基础层 =="
TOKEN=$(curl -sf "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/${BASE_IMAGE}:pull" | JSON_GET .token)
INDEX=$(curl -sf -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.docker.distribution.manifest.list.v2+json" \
  -H "Accept: application/vnd.oci.image.index.v1+json" \
  "https://registry-1.docker.io/v2/library/${BASE_IMAGE}/manifests/${BASE_TAG}")
AMD64=$(echo "$INDEX" | node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{const o=JSON.parse(d);const m=o.manifests.find(x=>x.platform.architecture==='amd64'&&(!x.platform.os||x.platform.os==='linux'));console.log(m.digest)})")
MANIFEST=$(curl -sf -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.docker.distribution.manifest.v2+json" \
  -H "Accept: application/vnd.oci.image.manifest.v1+json" \
  "https://registry-1.docker.io/v2/library/${BASE_IMAGE}/manifests/${AMD64}")
CFG_DIGEST=$(echo "$MANIFEST" | JSON_GET .config.digest)
LAYER_DIGEST=$(echo "$MANIFEST" | node -e "let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{const o=JSON.parse(d);console.log(o.layers[o.layers.length-1].digest)})")
curl -sfL -H "Authorization: Bearer $TOKEN" "https://registry-1.docker.io/v2/library/${BASE_IMAGE}/blobs/$CFG_DIGEST" -o base-config.json
curl -sfL -H "Authorization: Bearer $TOKEN" "https://registry-1.docker.io/v2/library/${BASE_IMAGE}/blobs/$LAYER_DIGEST" | gzip -dc > blobs/base.tar
echo "   基础层 $(du -h blobs/base.tar | cut -f1)"

echo "== 2/6 CA 证书（curl.se 的 Mozilla CA bundle） =="
mkdir -p .ca/etc/ssl/certs
curl -sfL "https://curl.se/ca/cacert.pem" -o .ca/etc/ssl/certs/ca-certificates.crt
[ -s .ca/etc/ssl/certs/ca-certificates.crt ] || { echo "CA 证书下载失败"; exit 1; }

echo "== 3/6 组装应用层 =="
tar -xf blobs/base.tar etc/passwd etc/group 2>/dev/null || true
[ -f etc/passwd ] || tar -xf blobs/base.tar ./etc/passwd ./etc/group
mkdir -p stage/usr/local/bin stage/etc/ssl/certs stage/usr/lib/ssl stage/data
cp "$BINARY" stage/usr/local/bin/bamboo && chmod 755 stage/usr/local/bin/bamboo
cp .ca/etc/ssl/certs/ca-certificates.crt stage/etc/ssl/certs/
cp stage/etc/ssl/certs/ca-certificates.crt stage/usr/lib/ssl/cert.pem
grep -q '^bamboo:' etc/passwd || echo 'bamboo:x:10001:10001::/data:/usr/sbin/nologin' >> etc/passwd
grep -q '^bamboo:' etc/group || echo 'bamboo:x:10001:' >> etc/group
cp etc/passwd stage/etc/passwd && cp etc/group stage/etc/group
tar -C stage --owner=0 --group=0 -cf blobs/app.tar .
echo "   应用层 $(du -h blobs/app.tar | cut -f1)"

echo "== 4/6 计算 diff_ids 并生成镜像 config =="
BASE_DIFF="sha256:$(sha256sum blobs/base.tar | cut -d' ' -f1)"
APP_DIFF="sha256:$(sha256sum blobs/app.tar | cut -d' ' -f1)"
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > image-config.json <<EOF
{
  "architecture": "amd64",
  "os": "linux",
  "config": {
    "User": "10001",
    "Env": [
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      "RUST_LOG=info",
      "BAMBOO_DATA_DIR=/data",
      "BAMBOO_PORT=9562",
      "BAMBOO_BIND=0.0.0.0",
      "SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt"
    ],
    "Entrypoint": ["bamboo"],
    "Cmd": ["serve", "--port", "9562", "--bind", "0.0.0.0", "--data-dir", "/data"],
    "ExposedPorts": { "9562/tcp": {} },
    "WorkingDir": "/app",
    "Healthcheck": {
      "Test": ["CMD-SHELL", "bash -c 'exec 3<>/dev/tcp/127.0.0.1/9562' && test -s /data/frontend/.frontend-manifest.json || exit 1"],
      "Interval": 30000000000,
      "Timeout": 10000000000,
      "StartPeriod": 10000000000,
      "Retries": 3
    }
  },
  "rootfs": { "type": "layers", "diff_ids": ["$BASE_DIFF", "$APP_DIFF"] },
  "history": [
    { "created": "$NOW", "created_by": "debian:bookworm-slim (amd64) official layer" },
    { "created": "$NOW", "created_by": "bamboo: binary + CA certs + user 10001" }
  ]
}
EOF
CFG_SHA=$(sha256sum image-config.json | cut -d' ' -f1)
cp image-config.json "$CFG_SHA.json"

echo "== 5/6 manifest.json + 规范 docker-save 布局（飞牛界面导入器按完整布局校验） =="
# docker save 的完整布局：每层一个 <id>/ 目录（layer.tar + VERSION + json），
# 加顶层 repositories 文件。层 ID 用 docker v1 公式（parentID + " " + diffID
# 的 sha256），与真引擎导出的结构一致；manifest 加载路径只按 manifest.json
# 的 Layers 路径取层，目录名本身不被校验。
V1_BASE=$(node -e "const c=require('crypto');console.log(c.createHash('sha256').update('sha256:'+process.argv[1].split(':')[1]).digest('hex'))" "${BASE_DIFF#sha256:}")
V1_APP=$(node -e "const c=require('crypto');console.log(c.createHash('sha256').update(process.argv[1]+' sha256:'+process.argv[2].split(':')[1]).digest('hex'))" "$V1_BASE" "${APP_DIFF#sha256:}")
mv blobs/base.tar "$V1_BASE/layer.tar" 2>/dev/null || { mkdir -p "$V1_BASE" && mv blobs/base.tar "$V1_BASE/layer.tar"; }
mv blobs/app.tar "$V1_APP/layer.tar" 2>/dev/null || { mkdir -p "$V1_APP" && mv blobs/app.tar "$V1_APP/layer.tar"; }
echo "1.0" > "$V1_BASE/VERSION"; echo "1.0" > "$V1_APP/VERSION"
cat > "$V1_BASE/json" <<EOF
{"id":"$V1_BASE","created":"$NOW","container_config":{"Cmd":[""]},"docker_version":"20.10.0","config":{}}
EOF
cat > "$V1_APP/json" <<EOF
{"id":"$V1_APP","parent":"$V1_BASE","created":"$NOW","container_config":{"Cmd":[""]},"docker_version":"20.10.0","config":{"User":"10001"}}
EOF
cat > repositories <<EOF
{ "bamboo-server": { "latest": "$V1_APP" } }
EOF
cat > manifest.json <<EOF
[{ "Config": "$CFG_SHA.json", "RepoTags": ["$IMAGE_TAG"], "Layers": ["$V1_BASE/layer.tar", "$V1_APP/layer.tar"] }]
EOF

echo "== 6/6 打包镜像 tar（未压缩——飞牛界面导入器不认 .tar.gz） =="
tar -cf "$OUT_DIR/bamboo-server-image.tar" manifest.json "$CFG_SHA.json" repositories "$V1_BASE" "$V1_APP"
echo "完成：$OUT_DIR/bamboo-server-image.tar ($(du -h "$OUT_DIR/bamboo-server-image.tar" | cut -f1))"
echo "导入：docker load -i bamboo-server-image.tar（或飞牛界面导入）→  $IMAGE_TAG"
