param([ValidateRange(1024,65535)][int]$Port=3108,[ValidateSet('debug','release')][string]$Profile='release')
$ErrorActionPreference='Stop'
$project=Get-Item -LiteralPath (Split-Path -Parent $PSScriptRoot)
$resolved=$project.ResolveLinkTarget($true)
$root=if($resolved){$resolved.FullName}else{$project.FullName}
$dir=Join-Path $root 'database'
$binary=Join-Path $root "target/$Profile/api-rs.exe"
if (-not (Test-Path -LiteralPath (Join-Path $dir 'vertex.db'))) { throw 'Base Vertex existante introuvable.' }
if (-not (Test-Path -LiteralPath $binary)) { throw "Compiler auparavant le profil $Profile de api-rs." }
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$Port)
try { $listener.Start() } catch { throw "Le port $Port est occupe. Choisir -Port avec un autre numero." } finally { $listener.Stop() }
$environment=@{VERTEX_DB_DIR=$dir;VERTEX_PORT=[string]$Port;VERTEX_BIND_HOST='127.0.0.1';VERTEX_PUBLIC_ORIGIN=$null;VERTEX_ADMIN_TOKEN=$null;VERTEX_ALLOW_DISCOVERY='0';VERTEX_LOCAL_CONTROL='1'}
$previous=@{}
try {
    foreach ($key in $environment.Keys) {
        $previous[$key]=[Environment]::GetEnvironmentVariable($key,'Process')
        if ($null -eq $environment[$key]) { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($key,$environment[$key],'Process') }
    }
    $stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
    $log=Join-Path $dir "panel-$stamp.log"
    $errors=Join-Path $dir "panel-$stamp.err.log"
    $process=Start-Process -FilePath $binary -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $errors -PassThru
    $state=@{pid=$process.Id;port=$Port;url="http://127.0.0.1:$Port/panel";database=$dir;log=$log;errors=$errors}
    [IO.File]::WriteAllText((Join-Path $dir 'panel-process.json'),($state | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    $state | ConvertTo-Json
} finally {
    foreach ($key in $previous.Keys) {
        if ($null -eq $previous[$key]) { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($key,$previous[$key],'Process') }
    }
}
