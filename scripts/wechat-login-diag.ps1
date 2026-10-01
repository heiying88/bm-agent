$base = "https://ilinkai.weixin.qq.com"
function New-WechatUin { [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes([string](Get-Random -Maximum 2147483647))) }
$headers = @{ "AuthorizationType" = "ilink_bot_token"; "X-WECHAT-UIN" = (New-WechatUin) }
$qr = Invoke-RestMethod -Method Get -Uri "$base/ilink/bot/get_bot_qrcode?bot_type=3" -Headers $headers -TimeoutSec 30
"=== fields ==="
$qr.PSObject.Properties | ForEach-Object { "{0} ({1})" -f $_.Name, $_.Value.GetType().Name }
"=== ret: $($qr.ret)"
"=== qrcode: $($qr.qrcode)"
"=== url: $($qr.url)"
$img = [string]$qr.qrcode_img_content
"=== img length: $($img.Length)"
"=== img first 100: " + $img.Substring(0, [Math]::Min(100, $img.Length))
"=== img last 30:  " + $img.Substring([Math]::Max(0, $img.Length - 30))
