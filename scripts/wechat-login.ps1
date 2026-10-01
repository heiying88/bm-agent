# scripts/wechat-login.ps1 —— 微信个人号 iLink 扫码登录，取得 bot_token
#
# 用途：首次为 Bamboo 的 wechat 渠道适配器获取 bot_token（适配器本体只在
# 会话过期 ret=-14 时自动扫码重登，首启 token 需要你手动取得并填进 connect.json）。
# 协议与 crates/app/bamboo-server/src/connect/platforms/wechat.rs 保持一致：
#   GET /ilink/bot/get_bot_qrcode?bot_type=3 → 扫码 → 轮询 get_qrcode_status
#   直至 status == "confirmed"，响应携带 bot_token 与按 bot 区分的 baseurl。
#
# 用法（PowerShell 或 cmd 均可）：
#   powershell -ExecutionPolicy Bypass -File scripts/wechat-login.ps1
#
# 扫码成功后把输出的 bot_token 填入 ~/.bamboo/connect.json 的
# platforms[type=wechat].token；若输出了 baseurl 且与默认网关不同，
# 一并填到该条目的 domain 字段。

$ErrorActionPreference = "Stop"
$base = "https://ilinkai.weixin.qq.com"

# 每个请求一次性 X-WECHAT-UIN：随机 uint32 十进制字符串的 base64（防重放）。
function New-WechatUin {
    [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes([string](Get-Random -Maximum 2147483647)))
}

Write-Host "== Bamboo 微信 iLink 扫码登录 ==" -ForegroundColor Cyan
$headers = @{ "AuthorizationType" = "ilink_bot_token"; "X-WECHAT-UIN" = (New-WechatUin) }
$qr = Invoke-RestMethod -Method Get -Uri "$base/ilink/bot/get_bot_qrcode?bot_type=3" -Headers $headers -TimeoutSec 30

if ($qr.ret -ne 0) {
    throw "get_bot_qrcode 失败 ret=$($qr.ret)：$($qr.errmsg)"
}

# 实测（2026-10）网关把登录链接放在 qrcode_img_content 字段（字段名有误导性，
# 实际是 URL 而非 base64 图片），`url` 字段为空。浏览器打开该页面即可看到
# 二维码；手机微信内直接点开链接则弹出确认。兼容性保留 base64 PNG 分支。
$image = [string]$qr.qrcode_img_content
if ($image -match '^https?://') {
    Write-Host "登录页面：$image"
    Write-Host "（已在默认浏览器打开；页面出现二维码后用手机微信扫一扫。"
    Write-Host "  也可以把上面这条链接发到手机微信里（如文件传输助手），直接点开确认。）"
    Start-Process $image
} elseif ($image.Length -gt 0) {
    $png = Join-Path $env:TEMP "bamboo-wechat-login-qr.png"
    [IO.File]::WriteAllBytes($png, [Convert]::FromBase64String($image))
    Start-Process $png
    Write-Host "二维码已打开（副本保存在 $png），请用手机微信扫一扫并确认登录。"
}
if ($qr.url) { Write-Host "登录链接：$($qr.url)" }

$qrcodeEscaped = [uri]::EscapeDataString([string]$qr.qrcode)
$deadline = (Get-Date).AddSeconds(480)
Write-Host "等待扫码确认（最长 480 秒）……"

while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 1
    $pollHeaders = @{ "AuthorizationType" = "ilink_bot_token"; "X-WECHAT-UIN" = (New-WechatUin) }
    try {
        $status = Invoke-RestMethod -Method Get `
            -Uri "$base/ilink/bot/get_qrcode_status?qrcode=$qrcodeEscaped" `
            -Headers $pollHeaders -TimeoutSec 30
    } catch {
        continue  # 瞬时失败不终止（截止时间兜底）
    }
    if ($status.ret -ne 0) { continue }

    if ($status.status -eq "confirmed") {
        Write-Host ""
        Write-Host "== 登录成功 ==" -ForegroundColor Green
        Write-Host "bot_token: $($status.bot_token)"
        if ($status.baseurl) { Write-Host "baseurl:   $($status.baseurl)" }
        Write-Host ""
        Write-Host "下一步：把 bot_token 填入 ~/.bamboo/connect.json："
        Write-Host @'
{
  "platforms": [
    {
      "type": "wechat",
      "token": "<上面的 bot_token>",
      "allow_from": []
    }
  ]
}
'@
        Write-Host "（若 baseurl 与 https://ilinkai.weixin.qq.com 不同，请把 baseurl 填入该条目的 "domain" 字段）"
        exit 0
    }
}

throw "等待扫码确认超时（480 秒）。重新运行本脚本再试。"
