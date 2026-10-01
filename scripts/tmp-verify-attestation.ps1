$dir = "C:\Users\29143\.bamboo"
$att = (Get-Content -Raw "$dir\config-section-layout-completion.json" | ConvertFrom-Json).completion.members
$mismatch = @(); $missing = @()
foreach ($p in $att.PSObject.Properties) {
  $f = Join-Path $dir $p.Name
  if (!(Test-Path $f)) { $missing += $p.Name; continue }
  $h = (Get-FileHash -Algorithm SHA256 $f).Hash.ToLower()
  if ($h -ne $p.Value.sha256) { $mismatch += $p.Name }
}
Write-Output ("missing: " + ($missing -join "; "))
Write-Output ("mismatched: " + ($mismatch -join "; "))
