param([ValidateRange(1024,65535)][int]$Port=3108)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$project=Get-Item -LiteralPath $root
$resolved=$project.ResolveLinkTarget($true)
if($resolved){$root=$resolved.FullName}
$lease=$null
try{
    $lease=[IO.File]::Open((Join-Path $root 'database/panel-launch.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $listener=Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue|Select-Object -First 1
    if($listener){
        $process=Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)"
        if($process.ExecutablePath -notlike "$root\target\*\api-rs.exe"){throw "Le port $Port appartient a une autre application."}
    }else{& (Join-Path $root 'scripts/panel.ps1') -Port $Port -Profile release|Out-Null}
    $until=[DateTime]::UtcNow.AddSeconds(60)
    do{
        try{$ready=(Invoke-WebRequest "http://127.0.0.1:$Port/health" -TimeoutSec 3).StatusCode -eq 200}catch{$ready=$false}
        if(!$ready){Start-Sleep -Milliseconds 400}
    }while(!$ready -and [DateTime]::UtcNow -lt $until)
    if(!$ready){throw 'Le panneau Vertex ne repond pas. Consulter database/panel-*.err.log.'}
    Start-Process "http://127.0.0.1:$Port/panel"
}catch{
    $log=Join-Path $root 'database/panel-launch.err.log'
    [IO.File]::AppendAllText($log,"$([DateTimeOffset]::Now.ToString('o')) $($_.Exception.Message)`r`n")
    throw
}finally{if($lease){$lease.Dispose()}}
