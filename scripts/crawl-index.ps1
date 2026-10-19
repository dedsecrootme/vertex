param(
    [ValidateSet('Start','Status','Stop')][string]$Action='Status',
    [ValidateRange(1,64)][int]$Workers=32,
    [ValidateSet('debug','release')][string]$Profile='release',
    [ValidateRange(1024,1048576)][int]$MaxStorageMiB=81920,
    [ValidateRange(1024,1048576)][int]$MinFreeMiB=51200
)
$ErrorActionPreference='Stop'
$project=Get-Item -LiteralPath (Split-Path -Parent $PSScriptRoot)
$resolved=$project.ResolveLinkTarget($true)
$root=if($resolved){$resolved.FullName}else{$project.FullName}
$dir=Join-Path $root 'database'
$binary=Join-Path $root "target/$Profile/crawler-rs.exe"
$statePath=Join-Path $dir 'drain-process.json'
$stopPath=Join-Path $dir 'drain.stop'
if ($Action -eq 'Stop') {
    [IO.File]::WriteAllText($stopPath,'stop')
    'Arret propre demande. Les URL non traitees restent dans la file.'
    exit
}
if ($Action -eq 'Status') {
    if (-not (Test-Path -LiteralPath $statePath)) { 'Aucun traitement lance par ce script.'; exit }
    $state=Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if($state.binary){$binary=$state.binary}
    $process=Get-CimInstance Win32_Process -Filter "ProcessId=$($state.pid)"
    [pscustomobject]@{Running=($process -and $process.ExecutablePath -eq $binary);Pid=$state.pid;Database=$dir;Log=$state.log;Errors=$state.errors}
    if (Test-Path -LiteralPath $state.log) { Get-Content -LiteralPath $state.log -Tail 12 }
    if (Test-Path -LiteralPath $state.errors) { Get-Content -LiteralPath $state.errors -Tail 12 }
    exit
}
if (-not (Test-Path -LiteralPath (Join-Path $dir 'vertex.db'))) { throw 'Index existant introuvable; aucun nouvel index vide ne sera lance.' }
if (-not (Test-Path -LiteralPath $binary)) { throw "Compiler auparavant crawler-rs, profil $Profile." }
$active=Get-CimInstance Win32_Process -Filter "Name='crawler-rs.exe'" | Where-Object { $_.ExecutablePath -like "$root\target\*" -and $_.CommandLine.Contains($dir) }
if ($active) { throw 'Un crawler de ce projet tourne deja. Verifier son etat avant de relancer.' }
if (Test-Path -LiteralPath $stopPath) { Remove-Item -LiteralPath $stopPath }
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
$log=Join-Path $dir "drain-$stamp.log"
$errors=Join-Path $dir "drain-$stamp.err.log"
$drive=[IO.DriveInfo]::new([IO.Path]::GetPathRoot($dir))
if($drive.AvailableFreeSpace -lt $MinFreeMiB*1MB){throw 'Reserve disque atteinte; aucun crawler lance.'}
$arguments=@('--drain','--db-dir',"`"$dir`"",'--workers',"$Workers",'--session-secs','300','--idle-secs','1','--max-storage-mib',"$MaxStorageMiB",'--min-free-mib',"$MinFreeMiB",'--stop-file',"`"$stopPath`"")
$process=Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $errors -PassThru
$state=@{pid=$process.Id;binary=$binary;profile=$Profile;database=$dir;log=$log;errors=$errors;started=(Get-Date).ToUniversalTime().ToString('o');mode='drain-existing';workers=$Workers;maxStorageMiB=$MaxStorageMiB;minFreeMiB=$MinFreeMiB}
[IO.File]::WriteAllText($statePath,($state | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$state | ConvertTo-Json
