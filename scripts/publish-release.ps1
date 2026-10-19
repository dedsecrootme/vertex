param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [Parameter(Mandatory)][ValidatePattern('^[a-z0-9-]+$')][string]$Bucket,
    [Parameter(Mandatory)][ValidatePattern('^[A-Fa-f0-9]{40}$')][string]$PublisherThumbprint
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
if ([IO.Path]::GetExtension($installer) -ne '.exe') { throw 'Un installateur Windows EXE est requis.' }
$config = Get-Content -LiteralPath (Join-Path $root 'apps/desktop/src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
if ($config.version -ne $Version) { throw 'La version doit correspondre a tauri.conf.json.' }
$signature = Get-AuthenticodeSignature -LiteralPath $installer
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $PublisherThumbprint) {
    throw 'Signature Authenticode valide du distributeur attendu requise. Aucun ancien installateur non signe ne sera publie.'
}
$file = Get-Item -LiteralPath $installer
if ($file.Length -le 0 -or $file.Length -gt 500000000) { throw 'Taille de fichier refusee.' }
$sha = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$name = "Vertex-$Version-x64-setup.exe"
$key = "releases/$Version/$sha/$name"
$manifest = @{ version = $Version; windows = @{ key = $key; filename = $name; size = $file.Length; sha256 = $sha } }
$temp = Join-Path ([IO.Path]::GetTempPath()) ("vertex-release-" + [guid]::NewGuid().ToString() + '.json')
$wrangler = Join-Path $root 'node_modules/.bin/wrangler.cmd'
try {
    [IO.File]::WriteAllText($temp, ($manifest | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    & $wrangler r2 object put "$Bucket/$key" --file $installer --content-type application/octet-stream --remote
    if ($LASTEXITCODE -ne 0) { throw 'Upload installateur echoue; manifeste inchange.' }
    # Publish the pointer last, after the immutable versioned installer exists.
    & $wrangler r2 object put "$Bucket/releases/stable.json" --file $temp --content-type application/json --remote
    if ($LASTEXITCODE -ne 0) { throw 'Publication du manifeste echouee.' }
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp }
}
