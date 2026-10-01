# Bamboo 浏览器运行时

server 会为每个打开的聊天浏览器在独立的 Node 进程中启动 `host.cjs`。host 使用固定版本的 `playwright-core` 包和 Chromium headless shell；Lotus 和 Nova 不是运行时依赖。

在 macOS 上本地开发（Node 20 或更高版本）：

```sh
cd browser-runtime
npm ci
./node_modules/.bin/playwright-core install --only-shell chromium
npm run test:runtime
```

`BAMBOO_BROWSER_NODE`、`BAMBOO_BROWSER_HOST_SCRIPT` 和 `BAMBOO_BROWSER_EXECUTABLE` 可以指向打包产物的绝对路径。没有这些覆盖时，Bamboo 使用 `PATH` 上的 `node`、本源码中的 `host.cjs`，以及 Playwright 已安装的浏览器缓存。`PLAYWRIGHT_BROWSERS_PATH` 可以选择另一个开发用浏览器缓存。桌面应用由 Bodhi 设置全部三个打包路径。
