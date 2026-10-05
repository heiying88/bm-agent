export PATH="$PATH:/c/Program Files/nodejs:/c/Program Files/7-Zip"

# 1. 改 Lotus-main 源码后：装依赖（首次）、构建
cd Lotus-main && npm ci && npm run build && cd ..

# 2. 把 dist 打包嵌入 bamboo-server crate（zip + 清单）
LOTUS_LOCAL_PATH=Lotus-main LOTUS_PACKAGE_NAME="@bigduu/lotus" \
  node scripts/frontend-package.cjs stage:prebuilt

# 3. 重编 bamboo（build.rs 会把新 zip 编进二进制）
cargo build -p bamboo-agent --bin bamboo